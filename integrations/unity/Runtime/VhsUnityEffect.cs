using System;
using System.Collections.Concurrent;
using System.Runtime.InteropServices;
using System.Threading;
using Unity.Collections;
using UnityEngine;
using UnityEngine.Rendering;

namespace VhsNext
{
    /// <summary>
    /// Per-camera state and asynchronous GPU-to-CPU bridge. Add this component to the
    /// camera that the URP or HDRP adapter should process.
    /// </summary>
    [DisallowMultipleComponent]
    [RequireComponent(typeof(Camera))]
    public sealed class VhsUnityEffect : MonoBehaviour
    {
        [SerializeField] private TextAsset paramsJson;
        [SerializeField] private uint frameRateNumerator = 60;
        [SerializeField] private uint frameRateDenominator = 1;
        [SerializeField] private bool premultipliedAlpha;
        [SerializeField, Range(1, 8)] private int maxQueuedFrames = 2;
        [SerializeField, Min(0.01f)] private float outputAspectWidth = 4f;
        [SerializeField, Min(0.01f)] private float outputAspectHeight = 3f;

        private static readonly int TargetAspectId = Shader.PropertyToID("_VhsTargetAspect");
        private static readonly int OutputAspectId = Shader.PropertyToID("_VhsOutputAspect");

        private sealed class Frame
        {
            public int Width;
            public int Height;
            public float[] Pixels;
        }

        private readonly ConcurrentDictionary<long, Frame> _readyFrames = new ConcurrentDictionary<long, Frame>();
        private readonly ConcurrentQueue<Frame> _outputFrames = new ConcurrentQueue<Frame>();
        private VhsNativeProcessor _processor;
        private Thread _worker;
        private RenderTexture _readbackTexture;
        private RenderTexture _presentationTexture;
        private Texture2D _outputTexture;
        private Material _aspectFitMaterial;
        private int _pendingFrames;
        private long _nextCaptureSequence;
        private int _width;
        private int _height;
        private volatile bool _closing;
        private string _workerError;
        private string _lastReportedError;
        private bool _readbackWarningLogged;
        private bool _textureErrorLogged;

        public string Status { get; private set; }

        private void OnEnable()
        {
            if (!Application.isPlaying)
                return;

            try
            {
                Shader aspectShader = Shader.Find("Hidden/VhsNext/AspectFit");
                if (aspectShader == null)
                    aspectShader = Resources.Load<Shader>("VhsNextAspectFit");
                if (aspectShader == null)
                    throw new InvalidOperationException("Could not find the VHS Next 4:3 aspect-fit shader.");
                _aspectFitMaterial = new Material(aspectShader)
                {
                    name = "VHS Next 4:3 aspect fit",
                    hideFlags = HideFlags.HideAndDontSave
                };

                _processor = new VhsNativeProcessor(
                    paramsJson != null ? paramsJson.text : "{}",
                    frameRateNumerator,
                    frameRateDenominator);
                _closing = false;
                Interlocked.Exchange(ref _nextCaptureSequence, 0);
                _worker = new Thread(ProcessFrames)
                {
                    IsBackground = true,
                    Name = "VHS Next frame processor"
                };
                _worker.Start();
                Status = "Ready";
            }
            catch (Exception error)
            {
                if (_processor != null)
                {
                    _processor.Dispose();
                    _processor = null;
                }
                Status = error.Message;
                Debug.LogError("VHS Next: " + error.Message, this);
            }
        }

        private void Update()
        {
            string workerError = Interlocked.Exchange(ref _workerError, null);
            if (!string.IsNullOrEmpty(workerError) && workerError != _lastReportedError)
            {
                _lastReportedError = workerError;
                Status = workerError;
                Debug.LogError("VHS Next: " + workerError, this);
            }

            Frame latest = null;
            while (_outputFrames.TryDequeue(out Frame completed))
                latest = completed;

            if (latest != null)
                UploadOutput(latest);
        }

        private void OnDisable()
        {
            _closing = true;
            if (_worker != null)
            {
                // Keep the staging texture alive until any queued readbacks have completed.
                AsyncGPUReadback.WaitAllRequests();
                _worker.Join();
                _worker = null;
            }
            if (_processor != null)
            {
                _processor.Dispose();
                _processor = null;
            }
            ReleaseTextures();
            if (_aspectFitMaterial != null)
            {
                Destroy(_aspectFitMaterial);
                _aspectFitMaterial = null;
            }
            _readyFrames.Clear();
            ClearQueue(_outputFrames);
            Interlocked.Exchange(ref _pendingFrames, 0);
        }

        /// <summary>Called by a render-pipeline adapter with the active camera color target.</summary>
        public void Record(CommandBuffer commandBuffer, RenderTexture cameraColor)
        {
            Record(commandBuffer, cameraColor, cameraColor);
        }

        /// <summary>Records a copy from the graph source and writes the processed frame to its destination.</summary>
        public void Record(CommandBuffer commandBuffer, Texture cameraColorSource, RenderTexture cameraColorDestination)
        {
            if (!isActiveAndEnabled || cameraColorSource == null || cameraColorDestination == null)
                return;

            if (cameraColorSource.dimension != TextureDimension.Tex2D)
            {
                if (!_readbackWarningLogged)
                {
                    Debug.LogWarning("VHS Next currently supports 2D camera targets only; XR and texture-array targets are skipped.", this);
                    _readbackWarningLogged = true;
                }
                return;
            }
            if (_processor == null || _closing)
            {
                TryPresentPassthrough(commandBuffer, cameraColorSource, cameraColorDestination);
                return;
            }
            if (!SystemInfo.supportsAsyncGPUReadback)
            {
                if (!_readbackWarningLogged)
                {
                    Debug.LogWarning("VHS Next requires AsyncGPUReadback on this device; the camera is left untouched.", this);
                    _readbackWarningLogged = true;
                }
                TryPresentPassthrough(commandBuffer, cameraColorSource, cameraColorDestination);
                return;
            }

            int width = cameraColorSource.width;
            int height = cameraColorSource.height;
            try
            {
                EnsureReadbackTexture(width, height);
            }
            catch (Exception error)
            {
                Status = error.Message;
                if (!_textureErrorLogged)
                {
                    Debug.LogError("VHS Next: " + error.Message, this);
                    _textureErrorLogged = true;
                }
                TryPresentPassthrough(commandBuffer, cameraColorSource, cameraColorDestination);
                return;
            }

            int pending = Interlocked.Increment(ref _pendingFrames);
            if (pending <= Mathf.Max(1, maxQueuedFrames))
            {
                long sequence = Interlocked.Increment(ref _nextCaptureSequence);
                try
                {
                    // Preserve the complete camera frame for the VHS simulation. The
                    // 4:3 aperture is applied after processing so the side bars stay black.
                    commandBuffer.Blit(cameraColorSource, _readbackTexture);
                    commandBuffer.RequestAsyncReadback(
                        _readbackTexture,
                        0,
                        TextureFormat.RGBAFloat,
                        request => OnReadback(request, width, height, sequence));
                }
                catch (Exception error)
                {
                    Interlocked.Exchange(ref _workerError, error.Message);
                    _readyFrames.TryAdd(sequence, new Frame { Width = width, Height = height });
                }
            }
            else
            {
                Interlocked.Decrement(ref _pendingFrames);
            }

            Texture presentationSource = _outputTexture != null ? (Texture)_outputTexture : cameraColorSource;
            PresentAspectFit(commandBuffer, presentationSource, cameraColorDestination);
        }

        private void OnReadback(AsyncGPUReadbackRequest request, int width, int height, long sequence)
        {
            if (_closing)
            {
                Interlocked.Decrement(ref _pendingFrames);
                return;
            }

            try
            {
                if (request.hasError)
                    throw new InvalidOperationException("GPU frame readback failed.");

                NativeArray<float> data = request.GetData<float>();
                float[] pixels = new float[data.Length];
                data.CopyTo(pixels);
                _readyFrames.TryAdd(sequence, new Frame { Width = width, Height = height, Pixels = pixels });
            }
            catch (Exception error)
            {
                Interlocked.Exchange(ref _workerError, error.Message);
                // Preserve sequence ordering even when one GPU readback fails.
                _readyFrames.TryAdd(sequence, new Frame { Width = width, Height = height });
            }
        }

        private void ProcessFrames()
        {
            long expectedSequence = 1;
            while (!_closing)
            {
                if (!_readyFrames.TryRemove(expectedSequence, out Frame input))
                {
                    Thread.Sleep(1);
                    continue;
                }
                expectedSequence++;

                try
                {
                    if (input.Pixels != null)
                    {
                        float[] output = new float[input.Pixels.Length];
                        _processor.Process(input.Pixels, output, input.Width, input.Height, premultipliedAlpha);
                        _outputFrames.Enqueue(new Frame { Width = input.Width, Height = input.Height, Pixels = output });
                    }
                }
                catch (Exception error)
                {
                    Interlocked.Exchange(ref _workerError, error.Message);
                }
                finally
                {
                    Interlocked.Decrement(ref _pendingFrames);
                }
            }
        }

        private void EnsureReadbackTexture(int width, int height)
        {
            if (_readbackTexture != null && _width == width && _height == height)
                return;

            if (_readbackTexture != null)
                AsyncGPUReadback.WaitAllRequests();
            ReleaseTextures();
            _width = width;
            _height = height;
            var descriptor = new RenderTextureDescriptor(width, height)
            {
                graphicsFormat = UnityEngine.Experimental.Rendering.GraphicsFormat.R32G32B32A32_SFloat,
                depthBufferBits = 0,
                msaaSamples = 1,
                sRGB = false,
                useMipMap = false,
                autoGenerateMips = false,
                dimension = TextureDimension.Tex2D,
                volumeDepth = 1
            };
            _readbackTexture = new RenderTexture(descriptor)
            {
                name = "VHS Next GPU readback",
                filterMode = FilterMode.Bilinear,
                wrapMode = TextureWrapMode.Clamp
            };
            if (!_readbackTexture.Create())
                throw new InvalidOperationException("Could not create an RGBAFloat readback texture.");

            _presentationTexture = new RenderTexture(descriptor)
            {
                name = "VHS Next 4:3 presentation",
                filterMode = FilterMode.Bilinear,
                wrapMode = TextureWrapMode.Clamp
            };
            if (!_presentationTexture.Create())
                throw new InvalidOperationException("Could not create the 4:3 presentation texture.");
        }

        private void TryPresentPassthrough(CommandBuffer commandBuffer, Texture source, RenderTexture destination)
        {
            try
            {
                EnsurePresentationTexture(source.width, source.height);
                PresentAspectFit(commandBuffer, source, destination);
            }
            catch (Exception error)
            {
                Status = error.Message;
                if (!_textureErrorLogged)
                {
                    Debug.LogError("VHS Next: " + error.Message, this);
                    _textureErrorLogged = true;
                }
                if (!ReferenceEquals(source, destination))
                    commandBuffer.Blit(source, destination);
            }
        }

        private void EnsurePresentationTexture(int width, int height)
        {
            if (_presentationTexture != null && _presentationTexture.width == width && _presentationTexture.height == height)
                return;

            if (_presentationTexture != null)
            {
                _presentationTexture.Release();
                Destroy(_presentationTexture);
                _presentationTexture = null;
            }

            var descriptor = new RenderTextureDescriptor(width, height)
            {
                graphicsFormat = UnityEngine.Experimental.Rendering.GraphicsFormat.R32G32B32A32_SFloat,
                depthBufferBits = 0,
                msaaSamples = 1,
                sRGB = false,
                useMipMap = false,
                autoGenerateMips = false,
                dimension = TextureDimension.Tex2D,
                volumeDepth = 1
            };
            _presentationTexture = new RenderTexture(descriptor)
            {
                name = "VHS Next 4:3 presentation",
                filterMode = FilterMode.Bilinear,
                wrapMode = TextureWrapMode.Clamp
            };
            if (!_presentationTexture.Create())
                throw new InvalidOperationException("Could not create the 4:3 presentation texture.");
        }

        private void PresentAspectFit(CommandBuffer commandBuffer, Texture source, RenderTexture destination)
        {
            if (_aspectFitMaterial == null)
            {
                if (ReferenceEquals(source, destination))
                    return;
                commandBuffer.Blit(source, destination);
                return;
            }

            float targetAspect = Mathf.Max(0.01f, outputAspectWidth) / Mathf.Max(0.01f, outputAspectHeight);
            float outputAspect = (float)destination.width / Mathf.Max(1, destination.height);
            _aspectFitMaterial.SetFloat(TargetAspectId, targetAspect);
            _aspectFitMaterial.SetFloat(OutputAspectId, outputAspect);

            if (ReferenceEquals(source, destination))
            {
                commandBuffer.Blit(source, _presentationTexture, _aspectFitMaterial);
                commandBuffer.Blit(_presentationTexture, destination);
            }
            else
            {
                commandBuffer.Blit(source, destination, _aspectFitMaterial);
            }
        }

        private void UploadOutput(Frame frame)
        {
            if (_outputTexture == null || _outputTexture.width != frame.Width || _outputTexture.height != frame.Height)
            {
                if (_outputTexture != null)
                    Destroy(_outputTexture);
                _outputTexture = new Texture2D(frame.Width, frame.Height, TextureFormat.RGBAFloat, false, true)
                {
                    name = "VHS Next processed frame",
                    filterMode = FilterMode.Bilinear,
                    wrapMode = TextureWrapMode.Clamp
                };
            }

            GCHandle pixelsPin = GCHandle.Alloc(frame.Pixels, GCHandleType.Pinned);
            try
            {
                _outputTexture.LoadRawTextureData(pixelsPin.AddrOfPinnedObject(), frame.Pixels.Length * sizeof(float));
                _outputTexture.Apply(false, false);
            }
            finally
            {
                pixelsPin.Free();
            }
            Status = "Processing";
        }

        private void ReleaseTextures()
        {
            if (_readbackTexture != null)
            {
                _readbackTexture.Release();
                Destroy(_readbackTexture);
                _readbackTexture = null;
            }
            if (_presentationTexture != null)
            {
                _presentationTexture.Release();
                Destroy(_presentationTexture);
                _presentationTexture = null;
            }
            if (_outputTexture != null)
            {
                Destroy(_outputTexture);
                _outputTexture = null;
            }
        }

        private static void ClearQueue(ConcurrentQueue<Frame> queue)
        {
            while (queue.TryDequeue(out _)) { }
        }
    }
}
