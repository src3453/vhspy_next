using UnityEditor;
using UnityEngine;

namespace VhsNext.Editor
{
    /// <summary>
    /// Inspector for <see cref="VhsUnityEffect"/>. The settings are the component's [SerializeField] fields
    /// (Undo, prefab overrides and multi-object editing work as usual); this editor only groups them and adds
    /// tube presets. In Play mode, edits are applied while the scene runs.
    /// </summary>
    [CustomEditor(typeof(VhsUnityEffect))]
    [CanEditMultipleObjects]
    public sealed class VhsUnityEffectEditor : UnityEditor.Editor
    {
        private struct Field
        {
            public readonly string Name;
            public readonly string Label;

            public Field(string name, string label)
            {
                Name = name;
                Label = label;
            }
        }

        private struct Group
        {
            public readonly string Title;
            public readonly Field[] Fields;

            public Group(string title, params Field[] fields)
            {
                Title = title;
                Fields = fields;
            }
        }

        private static readonly Group[] CameraGroups =
        {
            new Group("Optics",
                new Field("exposure_ev", "Exposure (EV)"),
                new Field("crosstalk", "Prism Crosstalk"),
                new Field("flare", "Flare"),
                new Field("registration_px", "Registration Error (px)"),
                new Field("shading", "Shading"),
                new Field("resolution_px", "Blur (px)")),
            new Group("Photoconductive Target",
                new Field("target_gamma", "Target Gamma"),
                new Field("dark_current", "Dark Current"),
                new Field("photo_lag", "Photoconductive Lag"),
                new Field("photo_lag_fields", "Lag Decay (fields)"),
                new Field("capacitive_lag", "Capacitive Lag"),
                new Field("bias_light", "Bias Light"),
                new Field("target_saturation", "Target Saturation"),
                new Field("hysteresis", "Hysteresis"),
                new Field("hysteresis_fields", "Hysteresis Relax (fields)"),
                new Field("blemishes", "Blemishes")),
            new Group("Beam and Burn-in",
                new Field("beam_current", "Beam Current"),
                new Field("act", "ACT (Anti-Comet-Tail)"),
                new Field("act_level", "ACT Level"),
                new Field("burn_rate", "Burn-in Rate (/s)"),
                new Field("burn_recovery", "Burn-in Recovery (/s)")),
            new Group("Tube Preamp",
                new Field("amp_drive", "Non-linearity"),
                new Field("amp_asymmetry", "Asymmetry"),
                new Field("noise_pct", "Noise (%)")),
            new Group("Tube Spread and Ageing",
                new Field("mismatch", "R/G/B Mismatch"),
                new Field("aging", "Ageing")),
            new Group("Processing",
                new Field("aperture", "Aperture Correction"),
                new Field("knee", "Knee Point"),
                new Field("knee_slope", "Knee Slope"),
                new Field("white_clip", "White Clip"),
                new Field("camera_gamma", "Gamma Correction"))
        };

        private struct Preset
        {
            public string Name;
            public float TargetGamma, PhotoLag, CapacitiveLag, DarkCurrent;
        }

        // Rough characteristics of the common photoconductor families (target gamma and lag).
        private static readonly Preset[] Presets =
        {
            new Preset { Name = "Vidicon", TargetGamma = 0.65f, PhotoLag = 0.3f, CapacitiveLag = 0.15f, DarkCurrent = 0.02f },
            new Preset { Name = "Plumbicon", TargetGamma = 0.95f, PhotoLag = 0.05f, CapacitiveLag = 0.08f, DarkCurrent = 0.005f },
            new Preset { Name = "Saticon", TargetGamma = 1.0f, PhotoLag = 0.03f, CapacitiveLag = 0.1f, DarkCurrent = 0.002f }
        };

        private SerializedProperty _backend, _source, _json, _standard, _seed, _camera;
        private SerializedProperty _fpsNum, _fpsDen, _premultiplied, _maxQueued, _aspectW, _aspectH;

        private void OnEnable()
        {
            _backend = serializedObject.FindProperty("backend");
            _source = serializedObject.FindProperty("paramsSource");
            _json = serializedObject.FindProperty("paramsJson");
            _standard = serializedObject.FindProperty("standard");
            _seed = serializedObject.FindProperty("seed");
            _camera = serializedObject.FindProperty("tubeCamera");
            _fpsNum = serializedObject.FindProperty("frameRateNumerator");
            _fpsDen = serializedObject.FindProperty("frameRateDenominator");
            _premultiplied = serializedObject.FindProperty("premultipliedAlpha");
            _maxQueued = serializedObject.FindProperty("maxQueuedFrames");
            _aspectW = serializedObject.FindProperty("outputAspectWidth");
            _aspectH = serializedObject.FindProperty("outputAspectHeight");
        }

        public override void OnInspectorGUI()
        {
            serializedObject.Update();

            if (Application.isPlaying && targets.Length == 1)
            {
                var effect = (VhsUnityEffect)target;
                EditorGUILayout.HelpBox("Status: " + (effect.Status ?? "-"), MessageType.None);
            }

            EditorGUILayout.LabelField("Processing", EditorStyles.boldLabel);
            EditorGUILayout.PropertyField(_backend);
            if (_backend.enumValueIndex == (int)VhsUnityEffect.ProcessingBackend.GpuComposite)
                EditorGUILayout.HelpBox("GPU composite: tube camera, composite encode and receiver decode only " +
                                        "(no tape, RF or TBC). Edits apply live in Play mode.", MessageType.Info);
            else if (Application.isPlaying)
                EditorGUILayout.HelpBox("Native CPU: each edit restarts the simulator in Play mode.", MessageType.Info);
            EditorGUILayout.PropertyField(_fpsNum, new GUIContent("Frame Rate Numerator"));
            EditorGUILayout.PropertyField(_fpsDen, new GUIContent("Frame Rate Denominator"));
            EditorGUILayout.PropertyField(_premultiplied);
            EditorGUILayout.PropertyField(_maxQueued);
            EditorGUILayout.PropertyField(_aspectW, new GUIContent("Output Aspect Width"));
            EditorGUILayout.PropertyField(_aspectH, new GUIContent("Output Aspect Height"));

            EditorGUILayout.Space();
            EditorGUILayout.LabelField("Parameters", EditorStyles.boldLabel);
            EditorGUILayout.PropertyField(_source, new GUIContent("Source"));
            if (_source.enumValueIndex == (int)VhsUnityEffect.ParamsSource.JsonAsset)
            {
                EditorGUILayout.PropertyField(_json, new GUIContent("Params JSON"));
                if (_json.objectReferenceValue == null)
                    EditorGUILayout.HelpBox("No JSON assigned: the simulator's defaults are used.", MessageType.Warning);
            }
            else
            {
                EditorGUILayout.PropertyField(_standard);
                EditorGUILayout.PropertyField(_seed);
                EditorGUILayout.Space();
                DrawCamera();
            }

            EditorGUILayout.Space();
            using (new EditorGUILayout.HorizontalScope())
            {
                GUILayout.FlexibleSpace();
                if (targets.Length == 1 && GUILayout.Button("Copy Params JSON", GUILayout.Width(140)))
                {
                    serializedObject.ApplyModifiedProperties();
                    EditorGUIUtility.systemCopyBuffer = ((VhsUnityEffect)target).BuildParamsJson();
                }
            }

            serializedObject.ApplyModifiedProperties();
        }

        private void DrawCamera()
        {
            SerializedProperty enabled = _camera.FindPropertyRelative("enabled");
            EditorGUILayout.PropertyField(enabled, new GUIContent("Tube Camera", enabled.tooltip));
            if (!enabled.boolValue && !enabled.hasMultipleDifferentValues)
                return;

            using (new EditorGUI.IndentLevelScope())
            {
                using (new EditorGUILayout.HorizontalScope())
                {
                    EditorGUILayout.PrefixLabel(new GUIContent("Tube Type", "Sets target gamma, lag and dark current."));
                    foreach (Preset preset in Presets)
                    {
                        if (GUILayout.Button(preset.Name, EditorStyles.miniButton))
                            ApplyPreset(preset);
                    }
                    if (GUILayout.Button("Defaults", EditorStyles.miniButton))
                        ResetCamera();
                }

                foreach (Group group in CameraGroups)
                {
                    string key = "VhsNext.CameraGroup." + group.Title;
                    bool open = SessionState.GetBool(key, group.Title == "Optics" || group.Title == "Beam and Burn-in");
                    open = EditorGUILayout.Foldout(open, group.Title, true);
                    SessionState.SetBool(key, open);
                    if (!open)
                        continue;
                    using (new EditorGUI.IndentLevelScope())
                    {
                        foreach (Field field in group.Fields)
                        {
                            SerializedProperty property = _camera.FindPropertyRelative(field.Name);
                            bool disabled = field.Name == "act_level" && !_camera.FindPropertyRelative("act").boolValue;
                            using (new EditorGUI.DisabledScope(disabled))
                                EditorGUILayout.PropertyField(property, new GUIContent(field.Label, property.tooltip));
                        }
                    }
                }
            }
        }

        private void ApplyPreset(Preset preset)
        {
            _camera.FindPropertyRelative("target_gamma").floatValue = preset.TargetGamma;
            _camera.FindPropertyRelative("photo_lag").floatValue = preset.PhotoLag;
            _camera.FindPropertyRelative("capacitive_lag").floatValue = preset.CapacitiveLag;
            _camera.FindPropertyRelative("dark_current").floatValue = preset.DarkCurrent;
        }

        private void ResetCamera()
        {
            serializedObject.ApplyModifiedProperties();
            Undo.RecordObjects(targets, "Reset Tube Camera");
            foreach (Object t in targets)
            {
                var so = new SerializedObject(t);
                SerializedProperty camera = so.FindProperty("tubeCamera");
                bool wasEnabled = camera.FindPropertyRelative("enabled").boolValue;
                camera.boxedValue = new VhsGpuCamera.Settings { enabled = wasEnabled };
                so.ApplyModifiedProperties();
            }
            serializedObject.Update();
        }
    }
}
