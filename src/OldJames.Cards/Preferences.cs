using System.Text.Json;
using Jupiter.UI;

namespace OldJames.Cards;

internal sealed class Preferences : IAccentPreferences
{
    public static readonly string DataDirectory = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Old James Cards");
    private static readonly string SettingsPath = Path.Combine(DataDirectory, "settings.json");
    public static string RulesetDirectory => Path.Combine(DataDirectory, "rulesets");

    public bool UseCustomAccent { get; set; }
    public string? CustomAccent { get; set; }
    public string? ModelFile { get; set; }
    public string? Game { get; set; }
    public int WindowWidth { get; set; }
    public int WindowHeight { get; set; }
    public bool Maximized { get; set; }

    public static Preferences Load()
    {
        Preferences settings;
        try
        {
            settings = JsonSerializer.Deserialize<Preferences>(File.ReadAllText(SettingsPath)) ?? new();
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or JsonException)
        {
            settings = new();
        }

        if (string.IsNullOrWhiteSpace(settings.ModelFile))
        {
            var seed = Path.Combine(AppContext.BaseDirectory, "model-path.txt");
            try
            {
                if (File.Exists(seed))
                    settings.ModelFile = File.ReadAllText(seed).Trim();
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                System.Diagnostics.Trace.TraceWarning(ex.Message);
            }
        }
        return settings;
    }

    public void Save()
    {
        try
        {
            Directory.CreateDirectory(DataDirectory);
            var temporary = Path.Combine(DataDirectory, $"settings-{Guid.NewGuid():N}.tmp");
            try
            {
                File.WriteAllText(temporary, JsonSerializer.Serialize(this));
                File.Move(temporary, SettingsPath, overwrite: true);
            }
            finally
            {
                File.Delete(temporary);
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            System.Diagnostics.Trace.TraceWarning(ex.Message);
        }
    }
}
