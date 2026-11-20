using Google.Protobuf;
using OldJames.Cards.Protocol;

namespace OldJames.Cards;

internal static class SavedGames
{
    private static string SavePath(string ruleset)
    {
        var custom = string.Equals(Path.GetDirectoryName(ruleset), Preferences.RulesetDirectory,
            StringComparison.OrdinalIgnoreCase);
        var name = Path.ChangeExtension(Path.GetFileName(ruleset), ".ojgame");
        return Path.Combine(Preferences.DataDirectory, "saved-games", custom ? "custom" : "built-in", name);
    }

    public static SavedGame? Load(string ruleset)
    {
        try
        {
            using var file = new FileStream(SavePath(ruleset), FileMode.Open, FileAccess.Read,
                FileShare.Read | FileShare.Delete);
            if (file.Length > 128 * 1024)
                throw new InvalidOperationException("The saved game is too large to restore.");
            return SavedGame.Parser.ParseFrom(file);
        }
        catch (FileNotFoundException) { return null; }
        catch (DirectoryNotFoundException) { return null; }
        catch (InvalidProtocolBufferException ex)
        {
            throw new InvalidOperationException("The save file is damaged.", ex);
        }
    }

    public static void Save(string ruleset, SavedGame game)
    {
        var path = SavePath(ruleset);
        var directory = Path.GetDirectoryName(path)!;
        Directory.CreateDirectory(directory);
        var temporary = Path.Combine(directory, $"{Guid.NewGuid():N}.tmp");
        try
        {
            using (var file = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write))
            {
                game.WriteTo(file);
                file.Flush(flushToDisk: true);
            }
            File.Move(temporary, path, overwrite: true);
        }
        finally
        {
            File.Delete(temporary);
        }
    }
}
