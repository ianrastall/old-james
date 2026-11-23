using Jupiter.UI;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using OldJames.Cards.Protocol;
using OldJames.Interop;
using System.Globalization;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using Windows.ApplicationModel.DataTransfer;
using Windows.Graphics;
using Windows.UI.ViewManagement;
using WinRT.Interop;
using CardAction = OldJames.Cards.Protocol.Action;

namespace OldJames.Cards;

public sealed partial class MainWindow : Window
{
    private const double CardScale = 1.5;

    private sealed record GameEntry(string File, string Json, GameInfo Info)
    {
        public override string ToString() => Info.Name;
    }

    private readonly Preferences _preferences;
    private readonly AccentService _accent;
    private readonly CardCore _core = new();
    private readonly AccessibilitySettings _accessibility = new();
    private readonly UISettings _systemColors = new();
    private readonly Dictionary<string, SvgImageSource> _images = new(StringComparer.Ordinal);
    private readonly Dictionary<(uint Zone, uint Card), Button> _targets = [];
    private readonly HashSet<uint> _drawCards = [];
    private GameEntry? _game;
    private Table? _table;
    private Start? _solitaireDeal;
    private Card? _selected;
    private CardAction? _hint;
    private ModelOpponent? _opponent;
    private CancellationTokenSource? _turn;
    private bool _loadingGames;
    private bool _canSave;
    private bool _closed;
    private string? _modelStatus;
    private double _cardSize = 76 * CardScale;

    internal MainWindow(Preferences preferences, AccentService accent)
    {
        InitializeComponent();
        _preferences = preferences;
        _accent = accent;
        _accent.Attach(Root);
        _systemColors.ColorValuesChanged += SystemColorsChanged;
        Root.ActualThemeChanged += (_, _) => Render();
        _accent.Changed += Render;
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "OldJames.ico"));
        var area = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Primary).WorkArea;
        var width = Math.Min(area.Width, preferences.WindowWidth > 0 ? preferences.WindowWidth : 1220);
        var height = Math.Min(area.Height, preferences.WindowHeight > 0 ? preferences.WindowHeight : 830);
        AppWindow.MoveAndResize(new RectInt32(area.X + (area.Width - width) / 2, area.Y + (area.Height - height) / 2, width, height));
        if (preferences.Maximized && AppWindow.Presenter is OverlappedPresenter presenter)
            presenter.Maximize();
        Closed += WindowClosed;
        Felt.SizeChanged += Felt_SizeChanged;
        Root.Loaded += Root_Loaded;
    }

    private void Root_Loaded(object sender, RoutedEventArgs args)
    {
        Root.Loaded -= Root_Loaded;
        LoadGames();
        OpenGame(resume: true);
    }

    private void WindowClosed(object sender, WindowEventArgs args)
    {
        _closed = true;
        CancelTurn();
        _opponent?.Dispose();
        _core.Dispose();
        _systemColors.ColorValuesChanged -= SystemColorsChanged;
        _accent.Changed -= Render;
        _accent.Detach(Root);
        _preferences.Maximized = AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Maximized };
        if (!_preferences.Maximized)
        {
            _preferences.WindowWidth = AppWindow.Size.Width;
            _preferences.WindowHeight = AppWindow.Size.Height;
        }
        _preferences.Save();
    }

    private void LoadGames()
    {
        _loadingGames = true;
        try
        {
            Directory.CreateDirectory(Preferences.RulesetDirectory);
            var builtIn = Path.Combine(AppContext.BaseDirectory, "rulesets");
            var paths = Directory.EnumerateFiles(builtIn, "*.json")
                .Concat(Directory.EnumerateFiles(Preferences.RulesetDirectory, "*.json"));
            var entries = new List<GameEntry>();
            var failures = new List<string>();
            foreach (var path in paths)
            {
                try
                {
                    if (new FileInfo(path).Length > 48 * 1024)
                        throw new InvalidOperationException("The ruleset exceeds 48 KB.");
                    var json = File.ReadAllText(path);
                    var reply = _core.Send(new Request { Inspect = new Inspect { Ruleset = json } });
                    entries.Add(new GameEntry(path, json, reply.Game));
                }
                catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or InvalidOperationException)
                {
                    failures.Add($"{Path.GetFileName(path)} · {ex.Message}");
                }
            }
            Games.ItemsSource = entries.OrderBy(g => g.Info.Multiplayer).ThenBy(g => g.Info.Name).ToArray();
            Games.SelectedItem = entries.FirstOrDefault(g => g.File == _preferences.Game)
                ?? entries.FirstOrDefault(g => g.Info.Name == "Klondike") ?? entries.FirstOrDefault();
            if (failures.Count > 0)
                ShowProblem("Some rulesets could not be loaded.", string.Join(Environment.NewLine, failures));
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            ShowProblem("Games could not be loaded.", ex.Message);
        }
        finally { _loadingGames = false; }
    }

    private void OpenGame(bool resume, Start? opening = null)
    {
        if (Games.SelectedItem is not GameEntry game) return;

        Reply? restored = null;
        string? restoreError = null;
        if (resume)
        {
            try
            {
                if (SavedGames.Load(game.File) is { } saved)
                {
                    restored = _core.Send(new Request { Restore = saved });
                    opening = saved.Start;
                }
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or InvalidOperationException)
            {
                restoreError = ex.Message;
            }
        }

        try
        {
            opening ??= new Start
            {
                Ruleset = game.Json,
                Seed = BitConverter.ToUInt64(RandomNumberGenerator.GetBytes(sizeof(ulong)))
            };
            var reply = restored ?? _core.Send(new Request { Start = opening });

            CancelTurn();
            _modelStatus = null;
            _selected = null;
            _hint = null;
            _drawCards.Clear();
            _game = game with { Info = reply.Game };
            _solitaireDeal = reply.Game.Multiplayer ? null : opening;
            _table = reply.Table;
            _canSave = restoreError is null;
            SaveProblem.IsOpen = !_canSave;
            if (restoreError is not null)
            {
                SaveProblem.Title = "The saved game could not be opened.";
                SaveProblem.Message = $"{restoreError} This table is temporary. Choose New game to replace the saved table.";
            }

            _preferences.Game = game.File;
            _preferences.Save();
            if (restored is null) SaveProgress();
            Render();
            Felt.ChangeView(0, 0, null, disableAnimation: true);
            StartAgent();
        }
        catch (InvalidOperationException ex) { ShowProblem("The game could not start.", ex.Message); }
    }

    private void NewDeal_Click(object sender, RoutedEventArgs args) => OpenGame(resume: false);

    private async void DealNumber_Click(object sender, RoutedEventArgs args)
    {
        if (_game is not { } game || _solitaireDeal is not { } current) return;
        var input = new TextBox { Header = "Deal number", Text = current.Seed.ToString(CultureInfo.InvariantCulture) };
        var message = new TextBlock { TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed };
        AutomationProperties.SetLiveSetting(message, AutomationLiveSetting.Polite);
        var content = new StackPanel { Spacing = 8 };
        content.Children.Add(new TextBlock
        {
            Text = "Use the same number to restart this deal, or enter another number. Playing a deal replaces this game's saved table.",
            TextWrapping = TextWrapping.Wrap
        });
        content.Children.Add(input);
        content.Children.Add(message);
        var dialog = new ContentDialog
        {
            Title = $"{game.Info.Name} deal",
            Content = content,
            PrimaryButtonText = "Play deal",
            SecondaryButtonText = "Copy number",
            CloseButtonText = "Cancel",
            DefaultButton = ContentDialogButton.Primary,
            XamlRoot = Root.XamlRoot
        };
        bool TryReadNumber(out ulong seed) => ulong.TryParse(input.Text.Trim(), NumberStyles.None,
            CultureInfo.InvariantCulture, out seed);
        void ShowMessage(string text)
        {
            message.Text = text;
            message.Visibility = text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        }

        input.TextChanged += (_, _) =>
        {
            var valid = TryReadNumber(out _);
            dialog.IsPrimaryButtonEnabled = dialog.IsSecondaryButtonEnabled = valid;
            ShowMessage(valid ? string.Empty : "Enter a whole number from 0 to 18446744073709551615.");
        };
        dialog.SecondaryButtonClick += (_, click) =>
        {
            click.Cancel = true;
            if (!TryReadNumber(out var number)) return;
            try
            {
                var data = new DataPackage();
                data.SetText(number.ToString(CultureInfo.InvariantCulture));
                Clipboard.SetContent(data);
                Clipboard.Flush();
                ShowMessage("Deal number copied.");
            }
            catch (COMException)
            {
                ShowMessage("The deal number could not be copied.");
            }
        };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary && !_closed && TryReadNumber(out var seed))
            OpenGame(resume: false, opening: seed == current.Seed ? current : new Start { Ruleset = game.Json, Seed = seed });
    }

    private void Games_SelectionChanged(object sender, SelectionChangedEventArgs args)
    {
        if (!_loadingGames) OpenGame(resume: true);
    }

    private void Undo_Click(object sender, RoutedEventArgs args)
    {
        if (_table?.CanUndo != true) return;
        try
        {
            _table = _core.Send(new Request { Undo = new Undo() }).Table;
            _selected = null;
            _hint = null;
            SaveProgress();
            Render();
        }
        catch (InvalidOperationException ex) { ShowProblem("Undo could not be applied.", ex.Message); }
    }

    private void Hint_Click(object sender, RoutedEventArgs args)
    {
        if (_table is not { HintAction: > 0 } table) return;
        var action = table.Actions.First(a => a.Id == table.HintAction);
        _selected = null;
        _hint = action;
        Render();
        ActionScroll.ChangeView(null, 0, null, disableAnimation: true);
        var source = table.Zones.First(zone => zone.Id == action.Source);
        var sourceCard = action.Card == 0 ? source.Cards.LastOrDefault()?.Id ?? 0 : action.Card;
        if (_targets.TryGetValue((source.Id, sourceCard), out var control))
            control.StartBringIntoView();
    }

    private async void Rules_Click(object sender, RoutedEventArgs args)
    {
        if (_game is null) return;
        await new ContentDialog
        {
            Title = _game.Info.Name,
            Content = new TextBlock { Text = _game.Info.Rules, TextWrapping = TextWrapping.Wrap },
            CloseButtonText = "Close",
            XamlRoot = Root.XamlRoot
        }.ShowAsync();
    }

    private async void Settings_Click(object sender, RoutedEventArgs args)
    {
        var dialog = new SettingsDialog(_preferences, _accent, WindowNative.GetWindowHandle(this)) { XamlRoot = Root.XamlRoot };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary || _closed) return;
        if (_preferences.ModelFile != dialog.SelectedModel)
        {
            CancelTurn();
            _opponent?.Dispose();
            _opponent = null;
            _preferences.ModelFile = dialog.SelectedModel;
            _preferences.Save();
            _modelStatus = null;
            StartAgent();
        }
    }

    private async void OpenRulesets_Click(object sender, RoutedEventArgs args)
    {
        try
        {
            Directory.CreateDirectory(Preferences.RulesetDirectory);
            var directory = await Windows.Storage.StorageFolder.GetFolderFromPathAsync(Preferences.RulesetDirectory);
            await Windows.System.Launcher.LaunchFolderAsync(directory);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            ShowProblem("The ruleset folder could not open.", ex.Message);
        }
    }

    private void Reload_Click(object sender, RoutedEventArgs args)
    {
        LoadGames();
        if (_game is null) OpenGame(resume: true);
    }

    private void Felt_SizeChanged(object sender, SizeChangedEventArgs args)
    {
        TableRows.MinWidth = Math.Max(0, args.NewSize.Width - 18);
        var size = Math.Clamp(Math.Floor((args.NewSize.Width - 84) / 10), 52, 112) * CardScale;
        if (size == _cardSize) return;
        _cardSize = size;
        Render();
    }

    private void SystemColorsChanged(UISettings sender, object args) => Root.DispatcherQueue.TryEnqueue(Render);

    private void Render()
    {
        if (_closed || _table is null || _game is null) return;
        var table = _table;
        Felt.Background = _accessibility.HighContrast
            ? Brush("JupiterWindowBrush") : new SolidColorBrush(ColorHelper.FromArgb(255, 22, 71, 53));
        TableRows.Children.Clear();
        _targets.Clear();
        foreach (var group in table.Zones.GroupBy(z => z.Row).OrderBy(g => g.Key))
        {
            if (group.First().Position is not null)
            {
                TableRows.Children.Add(RenderLayeredTableau(group));
                continue;
            }
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center };
            foreach (var zone in group) row.Children.Add(RenderZone(zone));
            TableRows.Children.Add(row);
        }
        RenderActions();
        LogHeader.Visibility = LogScroll.Visibility = _game.Info.Multiplayer ? Visibility.Visible : Visibility.Collapsed;
        Grid.SetRowSpan(ActionScroll, _game.Info.Multiplayer ? 1 : 3);
        UndoButton.IsEnabled = UndoMenu.IsEnabled = table.CanUndo;
        DealNumberMenu.IsEnabled = _solitaireDeal is not null;
        HintButton.Visibility = _game.Info.Multiplayer ? Visibility.Collapsed : Visibility.Visible;
        HintButton.IsEnabled = HintMenu.IsEnabled = table.HintAction != 0;
        HintText.Visibility = _hint is null ? Visibility.Collapsed : Visibility.Visible;
        HintText.Text = _hint is null ? string.Empty : $"Hint: {_hint.Label}";
        Status.Text = _modelStatus is null ? table.Status : $"{_modelStatus}   |   {table.Status}";
        PlayLog.Text = string.Join(Environment.NewLine, table.Events);
        LogScroll.ChangeView(null, double.MaxValue, null, disableAnimation: true);
    }

    private (double Width, double Height) CardDimensions() =>
        (_cardSize * (_game!.Info.Artwork == "bridge" ? 0.9 : 1), _cardSize * 1.4);

    private Grid RenderLayeredTableau(IEnumerable<Zone> group)
    {
        var zones = group.OrderBy(zone => zone.Position.Layer).ThenBy(zone => zone.Position.Column).ToArray();
        var (width, height) = CardDimensions();
        var columnStep = width / 2;
        var layerStep = height / 4;
        var tableau = new Grid
        {
            Width = width + zones.Max(zone => zone.Position.Column) * columnStep,
            Height = height + zones.Max(zone => zone.Position.Layer) * layerStep,
            HorizontalAlignment = HorizontalAlignment.Center
        };
        foreach (var zone in zones.Where(zone => zone.Cards.Count > 0))
        {
            var slot = RenderZone(zone);
            slot.HorizontalAlignment = HorizontalAlignment.Left;
            slot.VerticalAlignment = VerticalAlignment.Top;
            slot.Margin = new Thickness(zone.Position.Column * columnStep, zone.Position.Layer * layerStep, 0, 0);
            tableau.Children.Add(slot);
        }
        return tableau;
    }

    private FrameworkElement RenderZone(Zone zone)
    {
        var (width, height) = CardDimensions();
        var cards = new Grid();
        var spacingX = zone.Layout == Zone.Types.Layout.Hand
            ? (zone.Cards.All(c => c.Hidden) ? 8 : Math.Max(21, width * 0.46)) : 0;
        var spacingY = Math.Max(17, height * 0.23);
        var slots = zone.Layout == Zone.Types.Layout.Hand && zone.Label.StartsWith("Board", StringComparison.Ordinal)
            ? Math.Max(zone.Cards.Count, (int)zone.Count) : Math.Max(zone.Cards.Count, 1);
        var board = zone.Layout == Zone.Types.Layout.Hand && zone.Label.StartsWith("Board", StringComparison.Ordinal);
        if (board) spacingX = width + 6;
        cards.Width = width + (slots - 1) * spacingX;
        cards.Height = height + (zone.Layout == Zone.Types.Layout.Column ? (slots - 1) * spacingY : 0);
        cards.HorizontalAlignment = HorizontalAlignment.Left;
        for (var i = 0; i < slots; i++)
        {
            var card = i < zone.Cards.Count ? zone.Cards[i] : null;
            var cardId = card?.Id ?? 0;
            var positionY = zone.Layout == Zone.Types.Layout.Column ? spacingY * i : 0;
            var positionX = spacingX * i;
            var isTop = i == zone.Cards.Count - 1 || card is null;
            var targetAction = _selected is null ? null : _table!.Actions.FirstOrDefault(a => a.Card == _selected.Id && a.Destination == zone.Id);
            var directAction = _table!.Actions.FirstOrDefault(a => a.Source == zone.Id && a.Card == 0 && a.Selection is null);
            var hasChoice = cardId != 0 && _table.Actions.Any(a => a.Card == cardId);
            var canDraw = CanSelectForDraw(zone.Id, cardId);
            var markedForDraw = canDraw && _drawCards.Contains(cardId);
            var canClick = canDraw || hasChoice || (isTop && (targetAction is not null || directAction is not null));
            var hintSource = _hint is { } hint && hint.Source == zone.Id && (hint.Card == 0 ? isTop : hint.Card == cardId);
            var hintDestination = isTop && _hint?.Destination == zone.Id;
            var visual = Face(card, width, height, markedForDraw ||
                (isTop && targetAction is not null) || _selected?.Id == cardId && cardId != 0,
                hintSource || hintDestination);
            FrameworkElement element;
            var name = card is null ? zone.Label : $"{card.Name}, {zone.Label}";
            if (canClick)
            {
                var button = new Button
                {
                    Width = width, Height = height, MinWidth = 0, MinHeight = 0, Padding = new Thickness(0),
                    Background = new SolidColorBrush(Colors.Transparent), BorderThickness = new Thickness(0), Content = visual,
                    HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch
                };
                AutomationProperties.SetName(button, card is null || card.Hidden ? directAction?.Label ?? name : name);
                if (canDraw)
                    AutomationProperties.SetItemStatus(button, markedForDraw ? "Selected for replacement" : "Kept");
                ToolTipService.SetToolTip(button, card is null || card.Hidden ? directAction?.Label ?? name : name);
                button.Click += (_, _) => Tap(zone.Id, card, isTop);
                _targets.TryAdd((zone.Id, cardId), button);
                element = button;
            }
            else
            {
                AutomationProperties.SetName(visual, name);
                if (card is { Hidden: false }) ToolTipService.SetToolTip(visual, name);
                element = visual;
            }
            if (hintSource || hintDestination)
                AutomationProperties.SetItemStatus(element, hintSource ? "Suggested source" : "Suggested destination");
            element.Margin = new Thickness(positionX, positionY, 0, 0);
            element.HorizontalAlignment = HorizontalAlignment.Left;
            element.VerticalAlignment = VerticalAlignment.Top;
            cards.Children.Add(element);
        }
        if (zone.Position is not null)
            return cards;

        var label = new TextBlock
        {
            Text = zone.Count > 1 && zone.Label is "Stock" or "Waste"
                ? $"{zone.Label} ({zone.Count})" : zone.Label,
            Style = (Style)Application.Current.Resources["JupiterCaptionTextStyle"]
        };
        if (!_accessibility.HighContrast)
            label.Foreground = new SolidColorBrush(Colors.White);
        var items = new StackPanel { Spacing = 4 };
        items.Children.Add(label);
        items.Children.Add(cards);
        return items;
    }

    private Border Face(Card? card, double width, double height, bool highlighted, bool hinted)
    {
        var border = new Border { Width = width, Height = height, CornerRadius = new CornerRadius(6) };
        if (card is not null)
        {
            var format = _game!.Info.Artwork;
            var relative = card.Hidden ? $"{format}/backs/blue.svg" : $"{format}/faces/{card.Suit}/{card.Rank}.svg";
            if (!_images.TryGetValue(relative, out var image))
            {
                image = new SvgImageSource(new Uri($"ms-appx:///cards/{relative}"));
                _images.Add(relative, image);
            }
            border.Child = new Image { Source = image, Stretch = Stretch.Uniform };
        }
        if (hinted)
        {
            border.BorderThickness = new Thickness(4);
            border.BorderBrush = _accessibility.HighContrast ? Brush("JupiterPrimaryTextBrush") : new SolidColorBrush(_accent.Current);
        }
        else if (card is null || highlighted)
        {
            border.BorderThickness = new Thickness(highlighted ? 2 : 1);
            border.BorderBrush = highlighted ? new SolidColorBrush(_accent.Current) : _accessibility.HighContrast
                ? Brush("JupiterSeparatorBrush") : new SolidColorBrush(ColorHelper.FromArgb(255, 129, 169, 148));
        }
        return border;
    }

    private static Brush Brush(string key) => JupiterTheme.Brush(key);

    private bool CanSelectForDraw(uint zone, uint card) => card != 0 &&
        _table!.Actions.Any(action => action.Source == zone && action.Selection?.Cards.Contains(card) == true);

    private void Tap(uint zone, Card? card, bool isTop)
    {
        if (_table is null || _game is null) return;
        var actions = _table.Actions;
        if (card is not null && CanSelectForDraw(zone, card.Id))
        {
            if (!_drawCards.Add(card.Id)) _drawCards.Remove(card.Id);
            Render();
            if (_targets.TryGetValue((zone, card.Id), out var button)) button.Focus(FocusState.Programmatic);
            return;
        }
        if (_selected is not null && isTop)
        {
            var move = actions.FirstOrDefault(a => a.Card == _selected.Id && a.Destination == zone);
            if (move is not null) { Apply(move, _table.Revision, false); return; }
        }
        if (actions.FirstOrDefault(a => a.Source == zone && a.Card == 0 && a.Selection is null) is { } action)
        {
            Apply(action, _table.Revision, false);
            return;
        }
        if (card is { Id: > 0 })
        {
            var moves = actions.Where(a => a.Card == card.Id).ToArray();
            if (_game.Info.Multiplayer && moves.Length == 1) { Apply(moves[0], _table.Revision, false); return; }
            _selected = _selected?.Id == card.Id || moves.Length == 0 ? null : card;
            if (_hint?.Card != card.Id) _hint = null;
            Render();
            if (_targets.TryGetValue((zone, card.Id), out var control)) control.Focus(FocusState.Programmatic);
        }
    }

    private void RenderActions()
    {
        Actions.Children.Clear();
        ActionHeading.Text = "Actions";
        var table = _table!;
        if (table.AgentTurn)
        {
            if (_turn is null)
            {
                var retry = new Button { Content = "Resume opponent" };
                retry.Click += (_, _) => StartAgent();
                Actions.Children.Add(retry);
            }
            else Actions.Children.Add(new ProgressRing { IsActive = true, Width = 24, Height = 24, HorizontalAlignment = HorizontalAlignment.Left });
            return;
        }
        var draw = table.Actions.FirstOrDefault(a => a.Selection is not null && _drawCards.SetEquals(a.Selection.Cards));
        var shown = draw is null ? table.Actions.Where(a => a.Selection is null && (a.Card == 0 || a.Card == _selected?.Id)).ToArray() : [draw];
        if (draw is not null)
            ActionHeading.Text = "Select cards to replace";
        if (shown.Length == 0 && table.Actions.Count > 0)
            Actions.Children.Add(new TextBlock { Text = "Select a card to see its moves.", TextWrapping = TextWrapping.Wrap });
        foreach (var action in shown)
        {
            var text = action.Selection is not null && _drawCards.Count > 0 ? $"Draw {_drawCards.Count}" : action.Label;
            var label = new TextBlock { Text = text, TextWrapping = TextWrapping.Wrap };
            var button = new Button { Content = label, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left };
            button.Click += (_, _) => Apply(action, table.Revision, false);
            Actions.Children.Add(button);
        }
    }

    private void Apply(CardAction action, ulong revision, bool agent)
    {
        try
        {
            _table = _core.Send(new Request { Apply = new Apply { Action = action.Id, Revision = revision, Agent = agent } }).Table;
            _selected = null;
            _hint = null;
            _drawCards.Clear();
            if (!agent) Problem.IsOpen = false;
            SaveProgress();
            Render();
            if (!agent) StartAgent();
        }
        catch (InvalidOperationException ex)
        {
            if (agent) throw;
            ShowProblem("That action could not be played.", ex.Message);
        }
    }

    private void SaveProgress()
    {
        if (!_canSave || _game is null) return;
        try
        {
            var reply = _core.Send(new Request { Checkpoint = new Checkpoint() });
            SavedGames.Save(_game.File, reply.SavedGame);
            SaveProblem.IsOpen = false;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or InvalidOperationException)
        {
            SaveProblem.Title = "Game progress could not be saved.";
            SaveProblem.Message = ex.Message;
            SaveProblem.IsOpen = true;
        }
    }

    private void CancelTurn()
    {
        var turn = _turn;
        _turn = null;
        turn?.Cancel();
    }

    private void StartAgent()
    {
        if (_closed || _table?.AgentTurn != true || _turn is not null || _game is null) return;
        Problem.IsOpen = false;
        _turn = new CancellationTokenSource();
        _ = RunAgentAsync(_turn, _game.Info);
    }

    private async Task RunAgentAsync(CancellationTokenSource turn, GameInfo game)
    {
        try
        {
            while (_table?.AgentTurn == true && ReferenceEquals(_turn, turn))
            {
                var view = _core.Send(new Request { View = new View { Agent = true } }).Table;
                if (view.Actions.Count == 0) throw new InvalidOperationException("The opponent has no legal action.");
                CardAction move;
                if (view.Actions.Count == 1)
                {
                    move = view.Actions[0];
                    await Task.Delay(120, turn.Token);
                }
                else
                {
                    if (_opponent?.IsRunning != true)
                    {
                        _opponent?.Dispose();
                        _opponent = null;
                        _modelStatus = "Loading local model…";
                        Render();
                        // Start the server off the UI thread: process start-up work must not run
                        // inside XAML callbacks such as Loaded, which fail fast on re-entrancy.
                        var modelFile = _preferences.ModelFile;
                        var runtime = await Task.Run(() => ModelOpponent.OpenAsync(modelFile, turn.Token), turn.Token);
                        if (!ReferenceEquals(_turn, turn)) { runtime.Dispose(); return; }
                        _opponent = runtime;
                    }
                    _modelStatus = "Opponent thinking…";
                    Render();
                    var chosen = await _opponent.ChooseActionAsync(game, view, turn.Token);
                    move = view.Actions.First(a => a.Id == chosen);
                }
                if (!ReferenceEquals(_turn, turn)) return;
                Apply(move, view.Revision, true);
            }
        }
        catch (OperationCanceledException) when (turn.IsCancellationRequested) { }
        catch (Exception ex) when (ex is InvalidOperationException or HttpRequestException or IOException or System.ComponentModel.Win32Exception)
        {
            if (ReferenceEquals(_turn, turn)) ShowProblem("Opponent paused", ex.Message);
        }
        finally
        {
            if (ReferenceEquals(_turn, turn))
            {
                _turn = null;
                _modelStatus = null;
                Render();
            }
            turn.Dispose();
        }
    }

    private void ShowProblem(string title, string detail)
    {
        Problem.Title = title;
        Problem.Message = detail;
        Problem.IsOpen = true;
    }
}
