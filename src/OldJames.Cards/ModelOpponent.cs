using System.Diagnostics;
using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text.Json;
using OldJames.Cards.Protocol;

namespace OldJames.Cards;

internal sealed class ModelOpponent : IDisposable
{
    private readonly Process _server;
    private readonly HttpClient _http;
    private string? _lastError;

    private ModelOpponent(Process server, HttpClient http)
    {
        _server = server;
        _http = http;
        server.ErrorDataReceived += (_, e) =>
        {
            if (e.Data is { } line && (line.Contains("error", StringComparison.OrdinalIgnoreCase)
                || line.Contains("failed", StringComparison.OrdinalIgnoreCase)))
                Volatile.Write(ref _lastError, line);
        };
    }

    public bool IsRunning => !_server.HasExited;

    public static async Task<ModelOpponent> OpenAsync(string? modelFile, CancellationToken cancellation)
    {
        if (string.IsNullOrWhiteSpace(modelFile) || !File.Exists(modelFile))
            throw new InvalidOperationException("Choose a local GGUF model in Settings.");
        var executable = Path.Combine(AppContext.BaseDirectory, "runtime", "llama-server.exe");
        if (!File.Exists(executable))
            throw new InvalidOperationException("The local model runtime is missing. Reinstall Old James.");

        int port;
        using (var listener = new TcpListener(IPAddress.Loopback, 0))
        {
            listener.Start();
            port = ((IPEndPoint)listener.LocalEndpoint).Port;
        }
        var token = Convert.ToHexString(RandomNumberGenerator.GetBytes(24));
        var start = new ProcessStartInfo(executable)
        {
            CreateNoWindow = true,
            UseShellExecute = false,
            RedirectStandardError = true,
            RedirectStandardOutput = true,
            WorkingDirectory = Path.GetDirectoryName(executable)!
        };
        var threads = Math.Max(1, Environment.ProcessorCount / 2);
        foreach (var arg in new[]
        {
            "--model", Path.GetFullPath(modelFile), "--alias", "old-james",
            "--host", "127.0.0.1", "--port", port.ToString(), "--api-key", token,
            "--ctx-size", "4096", "--parallel", "1", "--threads", threads.ToString(),
            "--no-webui", "--reasoning-format", "deepseek", "--reasoning-effort", "low",
            "--reasoning-budget", "80"
        })
            start.ArgumentList.Add(arg);

        var server = new Process { StartInfo = start };
        var http = new HttpClient(new SocketsHttpHandler { UseProxy = false })
        {
            BaseAddress = new Uri($"http://127.0.0.1:{port}/"),
            Timeout = Timeout.InfiniteTimeSpan
        };
        http.DefaultRequestHeaders.Authorization = new AuthenticationHeaderValue("Bearer", token);
        var opponent = new ModelOpponent(server, http);
        try
        {
            if (!server.Start())
                throw new InvalidOperationException("The local model runtime could not start.");
            server.BeginErrorReadLine();
            server.BeginOutputReadLine();
            await opponent.WaitUntilReadyAsync(cancellation).ConfigureAwait(false);
            return opponent;
        }
        catch
        {
            opponent.Dispose();
            throw;
        }
    }

    private async Task WaitUntilReadyAsync(CancellationToken cancellation)
    {
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(cancellation);
        deadline.CancelAfter(TimeSpan.FromMinutes(4));
        try
        {
            while (!_server.HasExited)
            {
                try
                {
                    using var response = await _http.GetAsync("health", deadline.Token).ConfigureAwait(false);
                    if (response.IsSuccessStatusCode)
                        return;
                }
                catch (HttpRequestException)
                {
                    // The server has not yet bound its loopback port.
                }
                await Task.Delay(400, deadline.Token).ConfigureAwait(false);
            }
        }
        catch (OperationCanceledException) when (!cancellation.IsCancellationRequested)
        {
            throw new InvalidOperationException("The local model took too long to load.");
        }
        cancellation.ThrowIfCancellationRequested();
        throw new InvalidOperationException(Volatile.Read(ref _lastError) is { } detail
            ? $"The local model could not load. {detail}"
            : "The local model could not load. Check the GGUF file and available memory.");
    }

    public async Task<uint> ChooseActionAsync(GameInfo game, Table table, CancellationToken cancellation)
    {
        var legal = table.Actions.Select(action => action.Id).ToArray();
        var prompt = $"""
            You are playing {game.Name} against the person at the table. Play to win.
            Rules: {game.Rules}
            Your seat's position: {table.PlayerView}
            Public play in this deal: {string.Join("; ", table.Events.TakeLast(75))}
            Choose one legal action:
            {string.Join(Environment.NewLine, table.Actions.Select(a => $"{a.Id}: {a.Label}"))}
            Return the chosen action ID in the JSON field "action".
            """;
        var request = new
        {
            model = "old-james",
            messages = new[] { new { role = "user", content = prompt } },
            temperature = 0.25,
            max_tokens = 256,
            response_format = new
            {
                type = "json_schema",
                json_schema = new
                {
                    name = "card_action",
                    strict = true,
                    schema = new
                    {
                        type = "object",
                        properties = new { action = new { type = "integer", @enum = legal } },
                        required = new[] { "action" },
                        additionalProperties = false
                    }
                }
            }
        };
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(cancellation);
        deadline.CancelAfter(TimeSpan.FromMinutes(3));
        try
        {
            using var response = await _http.PostAsJsonAsync("v1/chat/completions", request, deadline.Token)
                .ConfigureAwait(false);
            if (!response.IsSuccessStatusCode)
            {
                Trace.TraceWarning("Model returned {0}: {1}", response.StatusCode,
                    await response.Content.ReadAsStringAsync(deadline.Token).ConfigureAwait(false));
                throw new InvalidOperationException($"The local model rejected the move request ({(int)response.StatusCode}).");
            }
            using var body = JsonDocument.Parse(await response.Content.ReadAsByteArrayAsync(deadline.Token)
                .ConfigureAwait(false));
            var content = body.RootElement.GetProperty("choices")[0].GetProperty("message").GetProperty("content").GetString();
            using var answer = JsonDocument.Parse(content ?? "");
            var action = answer.RootElement.GetProperty("action").GetUInt32();
            if (!legal.Contains(action))
                throw new InvalidOperationException("The local model returned an unavailable action.");
            return action;
        }
        catch (OperationCanceledException) when (!cancellation.IsCancellationRequested)
        {
            throw new InvalidOperationException("The local model took too long to choose an action.");
        }
        catch (Exception ex) when (ex is JsonException or KeyNotFoundException or IndexOutOfRangeException or FormatException)
        {
            throw new InvalidOperationException("The local model did not return a complete action.", ex);
        }
    }

    public void Dispose()
    {
        _http.Dispose();
        try
        {
            if (!_server.HasExited)
                _server.Kill(entireProcessTree: true);
        }
        catch (Exception ex) when (ex is InvalidOperationException or System.ComponentModel.Win32Exception)
        {
            Trace.TraceWarning(ex.Message);
        }
        finally
        {
            _server.Dispose();
        }
    }
}
