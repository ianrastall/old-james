using Jupiter.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Storage.Pickers;
using WinRT.Interop;

namespace OldJames.Cards;

public sealed partial class SettingsDialog : ContentDialog
{
    private readonly nint _window;

    internal SettingsDialog(Preferences preferences, AccentService accent, nint window)
    {
        InitializeComponent();
        _window = window;
        ModelPath.Text = preferences.ModelFile ?? "";
        AccentSetting.Service = accent;
    }

    internal string SelectedModel => ModelPath.Text.Trim().Trim('"');

    private async void Browse_Click(object sender, RoutedEventArgs e)
    {
        var picker = new FileOpenPicker { CommitButtonText = "Use model" };
        picker.FileTypeFilter.Add(".gguf");
        InitializeWithWindow.Initialize(picker, _window);
        if (await picker.PickSingleFileAsync() is { } file)
            ModelPath.Text = file.Path;
    }
}
