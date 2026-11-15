using Jupiter.UI;
using Microsoft.UI.Xaml;

namespace OldJames.Cards;

public sealed partial class App : Application
{
    private MainWindow? _window;

    public App() => InitializeComponent();

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        var settings = Preferences.Load();
        var accent = new AccentService(settings);
        _window = new MainWindow(settings, accent);
        _window.Activate();
    }
}
