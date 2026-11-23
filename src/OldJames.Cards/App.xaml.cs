using Jupiter.UI;
using Microsoft.UI.Xaml;

namespace OldJames.Cards;

public sealed partial class App : Application
{
    private MainWindow? _window;

    public App()
    {
        InitializeComponent();
        UnhandledException += (_, e) => RecordCrash(e.Exception);
        AppDomain.CurrentDomain.UnhandledException += (_, e) => RecordCrash(e.ExceptionObject as Exception);
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        var settings = Preferences.Load();
        var accent = new AccentService(settings);
        _window = new MainWindow(settings, accent);
        _window.Activate();
    }

    private static void RecordCrash(Exception? exception)
    {
        try
        {
            Directory.CreateDirectory(Preferences.DataDirectory);
            File.AppendAllText(Path.Combine(Preferences.DataDirectory, "crash.log"),
                $"{DateTimeOffset.Now:O}{Environment.NewLine}{exception}{Environment.NewLine}{Environment.NewLine}");
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            System.Diagnostics.Trace.TraceWarning(ex.Message);
        }
    }
}
