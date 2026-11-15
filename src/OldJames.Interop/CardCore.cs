using Google.Protobuf;
using Microsoft.Win32.SafeHandles;
using OldJames.Cards.Protocol;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace OldJames.Interop;

public sealed partial class CardCore : IDisposable
{
    private readonly EngineHandle _engine;
    private ulong _requestId;

    public CardCore()
    {
        if (AbiVersion() != 1)
            throw new InvalidOperationException("Old James and its card engine have incompatible versions.");
        _engine = new EngineHandle(Create());
        if (_engine.IsInvalid)
            throw new InvalidOperationException("The card engine could not start.");
    }

    public Reply Send(Request request)
    {
        request.Id = ++_requestId;
        var bytes = request.ToByteArray();
        var status = Call(_engine, bytes, checked((uint)bytes.Length), out var result);
        using var handle = new ResultHandle(result);
        if (status != 0 || handle.IsInvalid)
            throw new InvalidOperationException($"The card engine returned error {status}.");
        var buffer = ResultBytes(handle, out var length);
        if (buffer == IntPtr.Zero || length > 256 * 1024)
            throw new InvalidOperationException("The card engine returned an invalid reply.");
        var response = new byte[length];
        Marshal.Copy(buffer, response, 0, (int)length);
        var reply = Reply.Parser.ParseFrom(response);
        if (reply.Id != request.Id)
            throw new InvalidOperationException("The card engine returned a reply for another request.");
        if (reply.Error.Length > 0)
            throw new InvalidOperationException(reply.Error);
        return reply;
    }

    public void Dispose() => _engine.Dispose();

    private sealed class EngineHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        public EngineHandle(IntPtr pointer) : base(true) => SetHandle(pointer);
        protected override bool ReleaseHandle() { Destroy(handle); return true; }
    }

    private sealed class ResultHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        public ResultHandle(IntPtr pointer) : base(true) => SetHandle(pointer);
        protected override bool ReleaseHandle() { ResultFree(handle); return true; }
    }

    private const string Library = "old_james_cards_core";

    [LibraryImport(Library, EntryPoint = "oj_abi_version")]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    private static partial uint AbiVersion();

    [LibraryImport(Library, EntryPoint = "oj_create")]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    private static partial IntPtr Create();

    [LibraryImport(Library, EntryPoint = "oj_destroy")]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    private static partial void Destroy(IntPtr context);

    [LibraryImport(Library, EntryPoint = "oj_call")]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    private static partial uint Call(EngineHandle context, byte[] request, uint length, out IntPtr result);

    [LibraryImport(Library, EntryPoint = "oj_result_bytes")]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    private static partial IntPtr ResultBytes(ResultHandle result, out uint length);

    [LibraryImport(Library, EntryPoint = "oj_result_free")]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    private static partial void ResultFree(IntPtr result);
}
