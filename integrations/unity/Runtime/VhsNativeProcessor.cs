using System;
using System.Runtime.InteropServices;
using System.Text;

namespace VhsNext
{
    /// <summary>Thin managed wrapper around the stateful vhs-ffi C ABI.</summary>
    public sealed class VhsNativeProcessor : IDisposable
    {
        private const string LibraryName = "vhs_ffi";
        private const int ErrorBufferSize = 1024;
        private IntPtr _handle;

        [DllImport(LibraryName, EntryPoint = "vhs_processor_new", CallingConvention = CallingConvention.Cdecl)]
        private static extern IntPtr ProcessorNew(
            IntPtr paramsJson,
            UIntPtr paramsJsonLength,
            uint fpsNumerator,
            uint fpsDenominator,
            IntPtr errorBuffer,
            UIntPtr errorCapacity);

        [DllImport(LibraryName, EntryPoint = "vhs_processor_free", CallingConvention = CallingConvention.Cdecl)]
        private static extern void ProcessorFree(IntPtr handle);

        [DllImport(LibraryName, EntryPoint = "vhs_processor_last_error", CallingConvention = CallingConvention.Cdecl)]
        private static extern UIntPtr ProcessorLastError(IntPtr handle, IntPtr errorBuffer, UIntPtr errorCapacity);

        [DllImport(LibraryName, EntryPoint = "vhs_processor_process_rgba_f32", CallingConvention = CallingConvention.Cdecl)]
        private static extern int ProcessorProcess(
            IntPtr handle,
            IntPtr input,
            UIntPtr inputSamples,
            IntPtr output,
            UIntPtr outputSamples,
            UIntPtr width,
            UIntPtr height,
            uint alphaMode);

        public VhsNativeProcessor(string paramsJson, uint fpsNumerator, uint fpsDenominator)
        {
            if (fpsNumerator == 0 || fpsDenominator == 0)
                throw new ArgumentOutOfRangeException("fpsNumerator", "Frame rate numerator and denominator must be positive.");

            byte[] json = Encoding.UTF8.GetBytes(string.IsNullOrEmpty(paramsJson) ? "{}" : paramsJson);
            GCHandle jsonPin = GCHandle.Alloc(json, GCHandleType.Pinned);
            byte[] error = new byte[ErrorBufferSize];
            GCHandle errorPin = GCHandle.Alloc(error, GCHandleType.Pinned);
            try
            {
                _handle = ProcessorNew(
                    jsonPin.AddrOfPinnedObject(),
                    Size(json.Length),
                    fpsNumerator,
                    fpsDenominator,
                    errorPin.AddrOfPinnedObject(),
                    Size(error.Length));
            }
            finally
            {
                errorPin.Free();
                jsonPin.Free();
            }

            if (_handle == IntPtr.Zero)
                throw new InvalidOperationException(DecodeError(error, "Could not create the VHS processor."));
        }

        /// <summary>Processes one frame. Call frames in render order on one thread.</summary>
        public void Process(float[] input, float[] output, int width, int height, bool premultipliedAlpha)
        {
            if (_handle == IntPtr.Zero)
                throw new ObjectDisposedException(nameof(VhsNativeProcessor));
            if (input == null || output == null)
                throw new ArgumentNullException(input == null ? nameof(input) : nameof(output));
            if (ReferenceEquals(input, output))
                throw new ArgumentException("Input and output arrays must be separate.");
            if (width <= 0 || height <= 0 || (long)width * height * 4 != input.Length || output.Length < input.Length)
                throw new ArgumentException("Buffers must contain width * height * 4 float samples.");

            GCHandle inputPin = GCHandle.Alloc(input, GCHandleType.Pinned);
            GCHandle outputPin = GCHandle.Alloc(output, GCHandleType.Pinned);
            try
            {
                int status = ProcessorProcess(
                    _handle,
                    inputPin.AddrOfPinnedObject(),
                    Size(input.Length),
                    outputPin.AddrOfPinnedObject(),
                    Size(output.Length),
                    Size(width),
                    Size(height),
                    premultipliedAlpha ? 1u : 0u);
                if (status != 0)
                    throw new InvalidOperationException(GetLastError("VHS processing failed with status " + status + "."));
            }
            finally
            {
                outputPin.Free();
                inputPin.Free();
            }
        }

        public void Dispose()
        {
            if (_handle != IntPtr.Zero)
            {
                ProcessorFree(_handle);
                _handle = IntPtr.Zero;
            }
        }

        private string GetLastError(string fallback)
        {
            byte[] error = new byte[ErrorBufferSize];
            GCHandle pin = GCHandle.Alloc(error, GCHandleType.Pinned);
            try
            {
                ProcessorLastError(_handle, pin.AddrOfPinnedObject(), Size(error.Length));
            }
            finally
            {
                pin.Free();
            }
            return DecodeError(error, fallback);
        }

        private static UIntPtr Size(int value)
        {
            return UIntPtr.Size == 8 ? new UIntPtr((ulong)value) : new UIntPtr((uint)value);
        }

        private static string DecodeError(byte[] bytes, string fallback)
        {
            int end = Array.IndexOf(bytes, (byte)0);
            if (end < 0) end = bytes.Length;
            string message = Encoding.UTF8.GetString(bytes, 0, end);
            return string.IsNullOrEmpty(message) ? fallback : message;
        }
    }
}
