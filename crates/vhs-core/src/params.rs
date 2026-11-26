//! シミュレーションパラメータ。GUI とは JSON (serde) でやり取りする。
//! 全フィールドに `#[serde(default)]` を付け、古いプリセットも読めるようにしている。

use serde::{Deserialize, Serialize};

pub use crate::timing::TvStandard;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Params {
    /// テレビ方式 (信号のタイミング・色の符号化)
    pub standard: TvStandard,
    /// NTSC を 1 ライン 228 周期の非標準ラインにする (CGA・Apple II などのコンピュータの出力)。
    /// 画素の縞によるアーティファクトカラーがラインごとに反転しなくなる ([`crate::timing::NTSC_228`])
    pub ntsc_228: bool,
    /// 入力映像の画質調整 (パイプラインの先頭。カメラの有無に関わらずすべての映像に掛かる)
    pub input: InputParams,
    /// 撮像管カメラ (入力映像の画質調整の後)
    pub camera: CameraParams,
    pub encoder: EncoderParams,
    /// 送出側コンポジット信号の同期・色信号の劣化
    pub signal: SignalParams,
    /// 垂直帰線区間 (VBI) に重畳するデータ・試験信号
    pub vbi: VbiParams,
    pub tape: TapeParams,
    /// DV の記録・再生 (tape.format が DV / Digital8 のとき)
    pub dv: DvParams,
    pub transport: TransportParams,
    /// 再生機のタイムベースコレクタ (テープの後、受像機の前)
    pub tbc: TbcParams,
    /// VCR → TV の RF (アンテナ) 接続
    pub rf: RfParams,
    pub decoder: DecoderParams,
    /// デジタルキャプチャ (ADC・色差間引き・圧縮)
    pub capture: CaptureParams,
    /// 音声トラック (書き出し時に適用)
    pub audio: AudioParams,
    pub output: OutputParams,
    pub osd: OsdParams,
    /// ブラウン管の表示 (RetroArch slang シェーダー)。vhs-core 自体は使わず、GUI 側の GPU 処理で適用する
    pub crt: CrtParams,
    /// 静止画入力の配置・パン。vhs-core 自体は使わず、GUI 側で入力画像を作るときに使う
    pub image: ImageParams,
    /// 乱数シード (同じシードなら同じ劣化パターン)
    pub seed: u64,
}

impl Params {
    /// 使うテレビ方式のタイミング (`standard` と `ntsc_228` から決まる)
    pub fn tv(&self) -> &'static crate::timing::Standard {
        self.standard.spec_with(self.ntsc_228)
    }
}

/// 送出側の個別信号要素 (ジッタ・レベル減衰・雑音)。減衰は 0..100 [%]。
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SignalElementParams {
    /// 時間・位相の揺らぎ (信号要素ごとに単位が異なる)
    pub jitter: f32,
    /// 信号振幅の減衰 [%]
    pub attenuation_pct: f32,
    /// 加算雑音の rms [IRE]
    pub noise_ire: f32,
}

/// コンポジット信号内の同期・色信号ごとの劣化。
///
/// 信号は垂直帰線区間を含む全ラスタで作る。垂直同期の減衰・雑音は幅広パルス (垂直同期) そのものに掛かり、
/// 受像機の垂直同期分離 (積分回路) が同期を取り損ねると、垂直発振器が自走して画面が流れる。
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SignalParams {
    /// 垂直同期: ジッタ (同期に対する映像の上下ずれ) [ライン]、幅広パルスの振幅減衰、幅広パルスの雑音
    pub vertical_sync: SignalElementParams,
    /// 水平同期: ジッタ [サンプル]、パルス振幅減衰 (等化パルスも)、雑音
    pub horizontal_sync: SignalElementParams,
    /// カラーバースト: 位相ジッタ [度]、バースト振幅減衰、雑音
    pub color_burst: SignalElementParams,
    /// 色副搬送波 (有効映像中): 位相ジッタ [度]、色信号減衰、雑音
    pub chroma_subcarrier: SignalElementParams,
}

/// 垂直帰線区間 (VBI) に重畳する信号 (送出側)。画面の外にあるが、受像機の垂直同期が外れて画面が流れたとき、
/// 画面の上下位置をずらしたとき、くし形フィルタやドロップアウト補償が隣のラインを使うときに見えてくる。
/// テープにもそのまま記録される (ただし帯域の狭い VHS ではテレテキストは潰れる)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VbiParams {
    pub enabled: bool,
    /// クローズドキャプション (EIA-608、ライン 21 / 284)。NTSC のみ
    pub closed_caption: bool,
    /// 字幕として送る文 (改行か `\n` の 2 文字で行を分ける)。`*斜体*` `_下線_` `{red}` などで属性を変えられる
    pub caption_text: String,
    /// 字幕の送り方 (ポップオン・ロールアップ・ペイントオン)
    pub caption_mode: crate::caption::CaptionMode,
    /// 字幕を送るチャンネル (1〜4 = CC1〜CC4。CC3/CC4 はライン 284)
    pub caption_channel: u8,
    /// VITC (垂直間隔タイムコード、SMPTE 12M)。NTSC ライン 14/16、PAL ライン 19/21
    pub vitc: bool,
    /// VITS (挿入試験信号: 白バー・2T パルス・変調階段波・マルチバースト)
    pub vits: bool,
    /// 文字多重 (PAL: WST テレテキスト、NTSC: [`TeletextSystem`])
    pub teletext: bool,
    /// NTSC の文字多重の方式 (PAL は常に WST)
    pub teletext_system: TeletextSystem,
    /// 文字多重に使うライン数 (フィールドあたり)
    pub teletext_lines: u32,
    /// 画面サイズ・複製制御の識別信号 (PAL: WSS ライン 23、NTSC: IEC 61880 ライン 20)
    pub wss: bool,
}

impl Default for VbiParams {
    fn default() -> Self {
        Self {
            enabled: true,
            closed_caption: true,
            caption_text: r"VHS NEXT CLOSED CAPTION TEST\nTHE QUICK BROWN FOX\nJUMPS OVER THE LAZY DOG".into(),
            caption_mode: crate::caption::CaptionMode::PopOn,
            caption_channel: 1,
            vitc: false,
            vits: true,
            teletext: false,
            teletext_system: TeletextSystem::Nabts,
            teletext_lines: 8,
            wss: false,
        }
    }
}

/// NTSC の文字多重の方式
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TeletextSystem {
    /// NABTS (北米、10〜20 H / 273〜283 H の空きライン。内容は擬似乱数)
    #[default]
    Nabts,
    /// 日本の文字多重放送 (JTES / BT.653 System D)。14〜16・21 H / 277〜279・284 H のうち空いているライン
    Jtes,
}

/// 入力映像の画質調整。昔のビデオカメラの甘いレンズ・強い輪郭強調・色の癖などを作る。
/// 値はガンマ補正済み (表示用) の映像に対するもの。モデルの詳細は `adjust` モジュールを参照。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct InputParams {
    pub enabled: bool,
    /// ぼけ (レンズ・光学ローパス) のガウス σ [画素]
    pub blur_px: f32,
    /// 輪郭強調の量 (0 = なし、負で細部を削る)。輝度だけに掛かる
    pub sharpness: f32,
    /// 輪郭強調の半径 (ガウス σ) [画素]。大きいほどハローが太い
    pub sharpness_radius_px: f32,
    /// 輪郭強調の方向 (異方性)。縦方向の細部に掛ける割合 (0 = 横方向だけ, 1 = 縦横同じ, 間は線形)
    pub sharpness_vertical: f32,
    /// コアリング: これより小さい細部 (雑音) は強調しない (0..1 の振幅)
    pub sharpness_coring: f32,
    /// 強調で足す量の上限 (0 = 無制限)
    pub sharpness_limit: f32,
    /// 明るさ (加算, 白 = 1)
    pub brightness: f32,
    /// コントラスト (中間 0.5 を中心にした倍率)
    pub contrast: f32,
    /// ガンマ (1 より大きいと中間調が明るくなる)
    pub gamma: f32,
    /// 彩度 (倍率)
    pub saturation: f32,
    /// 色相の回転 [度]
    pub hue_deg: f32,
    /// 色温度 (-1 = 青っぽい, 1 = 赤っぽい)。明るさは保つ
    pub color_temp: f32,
    /// 色かぶり (-1 = 緑, 1 = マゼンタ)
    pub tint: f32,
}

impl Default for InputParams {
    fn default() -> Self {
        Self {
            enabled: false,
            blur_px: 0.0,
            sharpness: 0.0,
            sharpness_radius_px: 1.0,
            sharpness_vertical: 1.0,
            sharpness_coring: 0.0,
            sharpness_limit: 0.0,
            brightness: 0.0,
            contrast: 1.0,
            gamma: 1.0,
            saturation: 1.0,
            hue_deg: 0.0,
            color_temp: 0.0,
            tint: 0.0,
        }
    }
}

/// 撮像管 (ビジコン・プランビコン・サチコンなど) を使ったカラーカメラ。
/// 入力画像を「被写体の光」とみなし、3 管式カメラの光学系 → 光導電ターゲット → 電子ビーム読み出し →
/// 映像増幅・プロセス回路を通す。モデルの詳細は `camera` モジュールを参照。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CameraParams {
    pub enabled: bool,

    // --- 光学系 ---
    /// 露出 (アイリス) [EV]
    pub exposure_ev: f32,
    /// 色分解プリズムの混色 (隣の色の漏れ込み, 0..0.3)
    pub crosstalk: f32,
    /// レンズ・プリズムのフレア (画面平均の光が全体にかぶる量)。赤管は 2 倍
    pub flare: f32,
    /// レジストレーション誤差 [画素] (RGB 3 管の像の重ね合わせずれ)
    pub registration_px: f32,
    /// 周辺の感度低下 (シェーディング) 0..0.5
    pub shading: f32,
    /// レンズ・ターゲット・ビームスポットによるぼけ [画素]
    pub resolution_px: f32,

    // --- 光導電ターゲット ---
    /// ターゲットの γ (光量 → 信号電流)。ビジコン ≒ 0.65、プランビコン ≒ 0.95、サチコン ≒ 1.0
    pub target_gamma: f32,
    /// 暗電流 (白レベル比)
    pub dark_current: f32,
    /// 光導電性残像の量 (0..1)。トラップによる遅い応答
    pub photo_lag: f32,
    /// 光導電性残像の減衰時定数 [フィールド] (立ち上がりはこの半分)
    pub photo_lag_fields: f32,
    /// 容量性 (ビーム) 残像の強さ。低照度ほど読み残しが増える
    pub capacitive_lag: f32,
    /// バイアス光 (白レベル比)。残像を減らす
    pub bias_light: f32,
    /// ビーム電流の上限 (白レベル比)。超えた電荷は読み残されてコメットテールになる
    pub beam_current: f32,
    /// ターゲットに蓄えられる電荷の上限 (白レベル比)
    pub target_saturation: f32,
    /// ACT (アンチコメットテール): 帰線期間に過大な電荷を放電する
    pub act: bool,
    /// ACT が働き始めるレベル (白レベル比)
    pub act_level: f32,
    /// ターゲットのヒステリシス幅 (白レベル比)。明→暗と暗→明で残る像が変わる
    pub hysteresis: f32,
    /// ヒステリシスが解ける時定数 [フィールド]
    pub hysteresis_fields: f32,
    /// 焼き付きの進む速さ [/秒] (白を超えた部分の感度が落ちる)
    pub burn_rate: f32,
    /// 焼き付きが回復する速さ [/秒]
    pub burn_recovery: f32,
    /// ターゲットのしみ・きず (ブレミッシュ) の数
    pub blemishes: u32,

    // --- 映像増幅 (真空管プリアンプ) ---
    /// 非線形性の強さ (0 = 線形)
    pub amp_drive: f32,
    /// 正負の非対称性 (-1..1)。正で白側、負で黒側がより圧縮される
    pub amp_asymmetry: f32,
    /// プリアンプ雑音 (rms) [%]。周波数が高いほど大きい三角雑音
    pub noise_pct: f32,

    // --- 3 管のばらつき・劣化 ---
    /// RGB 各撮像管の特性のばらつき (0..1)。γ・残像・感度・黒レベルなどがずれる
    pub mismatch: f32,
    /// 撮像管の劣化 (0..1)。感度低下・ビーム電流低下・残像増加・焼き付き・しみ・雑音
    pub aging: f32,

    // --- プロセス回路 ---
    /// 輪郭補償 (アパーチャ補正) の量
    pub aperture: f32,
    /// ニーの開始レベル (白レベル比)
    pub knee: f32,
    /// ニーより上の傾き
    pub knee_slope: f32,
    /// ホワイトクリップ (白レベル比)
    pub white_clip: f32,
    /// ガンマ補正 (出力 = 入力^γ)。受像管 γ=2.2 の逆数 ≒ 0.45
    pub camera_gamma: f32,
}

impl Default for CameraParams {
    fn default() -> Self {
        Self {
            enabled: false,
            exposure_ev: 0.0,
            crosstalk: 0.03,
            flare: 0.02,
            registration_px: 0.4,
            shading: 0.08,
            resolution_px: 0.7,
            target_gamma: 0.95,
            dark_current: 0.005,
            photo_lag: 0.05,
            photo_lag_fields: 6.0,
            capacitive_lag: 0.08,
            bias_light: 0.0,
            beam_current: 2.0,
            target_saturation: 12.0,
            act: false,
            act_level: 1.5,
            hysteresis: 0.0,
            hysteresis_fields: 30.0,
            burn_rate: 0.02,
            burn_recovery: 0.05,
            blemishes: 0,
            amp_drive: 0.3,
            amp_asymmetry: 0.4,
            noise_pct: 0.8,
            mismatch: 0.2,
            aging: 0.0,
            aperture: 0.5,
            knee: 0.9,
            knee_slope: 0.3,
            white_clip: 1.08,
            camera_gamma: 0.45,
        }
    }
}

/// 放送/カメラ側のエンコーダ (NTSC は I/Q、PAL は U/V で色を載せる)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EncoderParams {
    /// コンポジット信号 (NTSC / PAL) に変換して、受像機で復調する。
    /// オフにすると映像はデジタルのまま (カメラ・OSD・DV・キャプチャだけを通す)。
    /// アナログのテープ・RF・TBC・受像機は飛ばす。DV なら FireWire で取り込んだのと同じ経路になる
    pub enabled: bool,
    /// 輝度帯域 [MHz]
    pub luma_bw_mhz: f32,
    /// I 信号帯域 [MHz]
    pub i_bw_mhz: f32,
    /// Q 信号帯域 [MHz]
    pub q_bw_mhz: f32,
    /// PAL の U/V 信号帯域 [MHz]
    pub uv_bw_mhz: f32,
    /// 色信号の振幅倍率
    pub chroma_gain: f32,
    /// カラーバーストの振幅 [%] (100 = 規格: NTSC 40 IRE p-p、PAL 300 mV p-p)。
    /// 受像機の ACC はバーストで色の濃さを正規化するので、大きくすると ACC ありで色が薄く、小さくすると濃くなる。
    /// CGA などのコンピュータは色信号と同じ回路でバーストを作るので、規格より大きいことがある
    pub burst_level_pct: f32,
}

impl Default for EncoderParams {
    fn default() -> Self {
        Self { enabled: true, luma_bw_mhz: 4.2, i_bw_mhz: 1.3, q_bw_mhz: 0.6, uv_bw_mhz: 1.3, chroma_gain: 1.0, burst_level_pct: 100.0 }
    }
}

/// 磁性体の伝達モデル
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MagneticModel {
    /// 線形 (飽和なし)
    Linear,
    /// tanh による無ヒステリシス飽和
    #[default]
    Tanh,
    /// Jiles-Atherton ヒステリシス (テープの種類は [`MagneticMedia`]、記録磁界は保磁力を単位に指定)
    JilesAtherton,
}

/// 磁性体の種類 (Jiles-Atherton の係数を選ぶ)
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MagneticMedia {
    /// コバルト被着酸化鉄 (VHS・ベータの標準テープ。Hc ≒ 56 kA/m、角形比 0.85)
    #[default]
    Cobalt,
    /// 高保磁力コバルト (S-VHS。Hc ≒ 72 kA/m、角形比 0.9)
    HighCobalt,
    /// メタル (8mm・Hi8 の MP テープ。Hc ≒ 120 kA/m、角形比 0.92)
    MetalParticle,
    /// 音声用の酸化鉄テープ (Chowdhury 2019 の係数。角形比 0.26 と丸いループ)
    FerricAudio,
}

/// テープのフォーマット (表示・既定値の選択用のラベル。信号処理は下の数値パラメータで決まる)
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TapeFormat {
    #[default]
    Vhs,
    Svhs,
    Beta,
    SuperBeta,
    Video8,
    Hi8,
    /// DV (MiniDV)。デジタル記録なので、アナログのテープ処理の代わりに `dv` の設定で処理する
    Dv,
    /// Digital8 (Hi8 のテープに DV 方式で記録)
    Digital8,
    /// 数値を個別に調整したもの
    Custom,
}

impl TapeFormat {
    /// デジタル記録 (DV 方式) のフォーマットか
    pub fn is_digital(self) -> bool {
        matches!(self, TapeFormat::Dv | TapeFormat::Digital8)
    }
}

/// DV の記録・再生。符号化 (25 Mbps 固定長) とテープのエラーの出方を決める
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DvParams {
    /// 動きのあるブロックを 2-4-8 DCT (フィールドの和と差) で符号化する
    pub dct_248: bool,
    /// 符号量の倍率 (1 = 規格どおりの 25 Mbps)。下げると複雑な絵柄の周りのブロックノイズが目立つ
    pub bitrate_scale: f32,
    /// マクロブロック (DIF ブロック) ごとの読み取りエラーの確率 [1/フレーム]
    pub error_rate: f32,
    /// ドロップアウト (テープの傷などで読めない区間。検出して修整される) の頻度 [回/秒]
    pub burst_rate: f32,
    /// ドロップアウト 1 回の平均の長さ [セグメント]
    pub burst_length: f32,
    /// バーストノイズ (データ化け) の頻度 [回/秒]
    pub noise_rate: f32,
    /// バーストノイズ 1 回の平均の長さ [マクロブロック]。読み取り順に続き、トラックをまたぐこともある
    pub noise_length: f32,
    /// バーストノイズの区間内のバイト誤り率 (0..1)
    pub noise_density: f32,
    /// 誤り訂正 (内符号)。マクロブロックごとに数バイトまでの誤りは直し、それ以上は検出して修整に回す。
    /// オフだと誤りはすべて化けたまま復号される
    pub ecc: bool,
    /// 訂正しきれない誤りを検出し損ねる確率 (化けたまま復号される)
    pub ecc_miss: f32,
    /// ヘッドの目詰まりの頻度 [回/分]。片方なら 1 本おきのトラック、両方なら全体が読めない
    pub head_clog_rate: f32,
    /// 目詰まり 1 回の平均の長さ [フレーム]
    pub head_clog_frames: f32,
    /// エラー修整 (読めないブロックを前のフレームで埋める)。オフだと壊れたブロックがそのまま出る
    pub concealment: bool,
}

impl Default for DvParams {
    fn default() -> Self {
        Self {
            dct_248: true,
            bitrate_scale: 1.0,
            error_rate: 0.0,
            burst_rate: 0.0,
            burst_length: 4.0,
            noise_rate: 0.0,
            noise_length: 20.0,
            noise_density: 0.05,
            ecc: true,
            ecc_miss: 0.05,
            head_clog_rate: 0.0,
            head_clog_frames: 8.0,
            concealment: true,
        }
    }
}

/// 輝度の FM 記録のモデル
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LumaModel {
    /// ベースバンドの近似: 帯域制限 + 加算ノイズ + 確率的なベアディング (軽い)
    #[default]
    Simple,
    /// FM の変調・伝送路・復調を実際に計算する (搬送波周波数・RF レベルで画質が決まる)
    Fm,
}

/// 記録・テープ媒体・再生の信号経路
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TapeParams {
    /// false ならテープを通さない (放送をそのまま受像)
    pub enabled: bool,
    /// フォーマット (GUI の既定値の選択に使うラベル)
    pub format: TapeFormat,
    /// 輝度の FM 記録のモデル
    pub luma_model: LumaModel,

    // --- FM 記録 (luma_model = Fm のときに使う) ---
    /// シンクチップの搬送波周波数 [MHz]
    pub fm_sync_mhz: f32,
    /// ホワイトピークの搬送波周波数 [MHz]
    pub fm_white_mhz: f32,
    /// 再生伝送路 (ヘッド・テープ・再生アンプ) の帯域 [MHz]。狭いほど白のエッジでベアディングが出る
    pub fm_channel_mhz: f32,
    /// RF ノイズ (伝送路の帯域内の rms、搬送波振幅 1 に対する比)
    pub fm_noise: f32,
    /// ビデオヘッドとテープの相対速度 (書き込み速度) [m/s]。VHS NTSC ≒ 5.8、PAL ≒ 4.85、8mm NTSC ≒ 3.75。
    /// 同じスペーシングでも遅いほど波長が短くなり損失が大きい
    pub writing_speed_mps: f32,
    /// 再生 EQ が想定していない余分なヘッド・テープ間隔 [µm] (ヘッドの汚れ・テープの浮きや摩耗)。
    /// 損失は 54.6 d/λ [dB] で、搬送波の高い白ほど大きく落ちて S/N が下がる。0 で無効
    pub head_spacing_um: f32,
    /// 低域変換クロマの周波数 [kHz]
    pub color_under_khz: f32,
    /// 低域変換クロマの記録レベル (バースト振幅のとき、搬送波振幅 1 に対する比)。輝度へのモアレになる
    pub color_under_level: f32,

    // --- 輝度 (FM 記録) 経路 ---
    /// 輝度記録帯域 [MHz] (VHS SP ≒ 3.0, LP/EP はさらに狭い)
    pub luma_bw_mhz: f32,
    /// 記録エンファシス量 [dB]
    pub emphasis_db: f32,
    /// エンファシスのコーナー周波数 [MHz]
    pub emphasis_mhz: f32,
    /// エンファシス後のホワイトクリップ [IRE]
    pub white_clip_ire: f32,
    /// FM 過変調で反転が起きる閾値 [IRE] (ベアディング。Simple モデルのみ)
    pub bearding_threshold_ire: f32,
    /// ベアディングの強さ
    pub bearding_strength: f32,
    /// FM 復調ノイズ (rms) [IRE] (Simple モデルのみ。Fm モデルは fm_noise)
    pub luma_noise_ire: f32,
    /// 再生ピーキング [dB] (負で甘く、正でシャープ)
    pub sharpness_db: f32,

    // --- 色 (低域変換記録) 経路 ---
    /// 低域変換クロマの帯域 (片側) [kHz]
    pub chroma_bw_khz: f32,
    /// クロマ AM ノイズ (rms) [IRE]
    pub chroma_noise_ire: f32,
    /// 再生時のクロマ位相誤差 (rms) [deg]
    pub chroma_phase_noise_deg: f32,
    /// 輝度に対するクロマの遅延 [サンプル] (Y/C 遅延誤差)。フィルタの群遅延は VCR の Y/C 遅延の整合で打ち消され、
    /// 画面上に残る色のずれはこの値だけになる (正で色が右へ)
    pub chroma_delay_samples: i32,
    /// 磁性体モデル
    pub magnetic_model: MagneticModel,
    /// 磁性体の種類 (J-A のときに使う)
    pub magnetic_media: MagneticMedia,
    /// J-A: FM 搬送波の記録磁界の振幅 [保磁力 Hc の倍数]。2 で再生 RF が基準の大きさ。
    /// 小さいと RF が弱くノイズが増え、大きいと飽和して搬送波と色の相互変調が増える。色のバイアスでもある
    pub rf_drive_hc: f32,
    /// 磁気飽和の起点となるクロマ振幅 [IRE]
    pub saturation_ire: f32,
    /// FM 輝度の RF の飽和レベル (記録搬送波の振幅 = 1 に対する比)。
    /// 搬送波と低域変換クロマの相互変調 (輝度のモアレ) と搬送波の歪みになる (磁性体モデルが線形なら無効)
    pub rf_saturation: f32,

    // --- ドロップアウト ---
    /// 1 ラインあたりのドロップアウト発生確率
    pub dropout_rate: f32,
    /// ドロップアウト平均長 [サンプル]
    pub dropout_length: f32,
    /// ドロップアウトでテープがヘッドから浮く量の目安 [µm] (実際は 0.3 倍から、平均 1.3 倍の裾の長い分布)。
    /// 浅いものは搬送波の高い白い部分だけが崩れ、深いものは RF が消える
    pub dropout_depth_um: f32,
    /// DOC がドロップアウトとして検出する RF レベルの低下 [dB] (中心搬送波で)。これより浅いものはそのまま再生される
    pub doc_threshold_db: f32,
    /// ドロップアウト補償 (1H 前のラインで置換)
    pub dropout_compensation: bool,

    // --- テープの物理的な傷み ---
    /// しわ・折れ目 (クリース) の発生頻度 [回/分]
    pub crease_rate: f32,
    /// 1 回のしわが画面に現れる長さ [フィールド]
    pub crease_fields: f32,
    /// ヘッドの目詰まり (画面全体が一瞬ノイズに埋もれる) の発生頻度 [回/分]
    pub head_clog_rate: f32,
}

impl Default for TapeParams {
    fn default() -> Self {
        Self {
            enabled: true,
            format: TapeFormat::Vhs,
            luma_model: LumaModel::Simple,
            fm_sync_mhz: 3.4,
            fm_white_mhz: 4.4,
            fm_channel_mhz: 5.5,
            fm_noise: 0.05,
            writing_speed_mps: 5.8,
            head_spacing_um: 0.0,
            color_under_khz: 629.0,
            color_under_level: 0.1,
            luma_bw_mhz: 3.0,
            emphasis_db: 10.0,
            emphasis_mhz: 0.6,
            white_clip_ire: 150.0,
            bearding_threshold_ire: 180.0,
            bearding_strength: 1.0,
            luma_noise_ire: 2.0,
            sharpness_db: 0.0,
            chroma_bw_khz: 500.0,
            chroma_noise_ire: 1.0,
            chroma_phase_noise_deg: 3.0,
            chroma_delay_samples: 3,
            magnetic_model: MagneticModel::Tanh,
            magnetic_media: MagneticMedia::Cobalt,
            rf_drive_hc: 2.0,
            saturation_ire: 50.0,
            rf_saturation: 1.5,
            dropout_rate: 0.002,
            dropout_length: 40.0,
            dropout_depth_um: 0.8,
            doc_threshold_db: 20.0,
            dropout_compensation: true,
            crease_rate: 0.0,
            crease_fields: 20.0,
            head_clog_rate: 0.0,
        }
    }
}

/// テープ走行系・ヘッド (時間軸誤差)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TransportParams {
    /// ライン単位の時間軸ジッタ (rms) [サンプル]
    pub jitter_samples: f32,
    /// ジッタのライン間相関 (0..1)
    pub jitter_correlation: f32,
    /// ワウ・フラッタ振幅 [サンプル]
    pub wow_samples: f32,
    /// ワウ・フラッタ周波数 [Hz]
    pub wow_hz: f32,
    /// ヘッド切替ノイズが及ぶライン数 (画面下端)
    pub head_switch_lines: usize,
    /// ヘッド切替による水平ずれ [サンプル]
    pub head_switch_offset: f32,
    /// ヘッド切替ラインのノイズ量
    pub head_switch_noise: f32,
    /// 画面上部のスキュー (テープ張力誤差) [サンプル]
    pub skew_samples: f32,
    /// トラッキングずれ (0..1)。ノイズバーが現れる
    pub tracking_error: f32,
    /// ノイズバーの移動速度 [ライン/フィールド]
    pub tracking_speed: f32,
    /// 垂直方向の揺れ (rms) [ライン]。安物デッキの V ジッタ
    pub vertical_jitter: f32,
}

impl Default for TransportParams {
    fn default() -> Self {
        Self {
            jitter_samples: 0.4,
            jitter_correlation: 0.9,
            wow_samples: 0.0,
            wow_hz: 8.0,
            head_switch_lines: 5,
            head_switch_offset: 10.0,
            head_switch_noise: 1.0,
            skew_samples: 0.0,
            tracking_error: 0.0,
            tracking_speed: 1.5,
            vertical_jitter: 0.0,
        }
    }
}

/// タイムベースコレクタ (TBC)。再生信号をデジタル化し、ラインごとに水平同期を検出して
/// 基準のクロックで読み出し直すことで、ジッタ・スキュー・ヘッド切替の曲がりを取り除く。
/// 同期を検出できない・誤検出したラインは補正を誤り、横ずれ (TBC エラー) として残る
/// (同期を作り直すので受像機の AFC ではもう直せない)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TbcParams {
    pub enabled: bool,
    /// 補正できる範囲 (ラインメモリの余裕) [±サンプル]。超えた分はそのまま残る
    pub window_samples: f32,
    /// 同期を探すゲートの幅 [±サンプル]。前のラインの同期位置からこれ以上離れた同期は無視する
    pub gate_samples: f32,
    /// 同期を見失ってからゲートを開いて再捕捉するまでのライン数 (それまでは前の補正量を保持)
    pub reacquire_lines: u32,
    /// 同期の誤検出 (ノイズで誤トリガ) の確率 [1/ライン]
    pub error_rate: f32,
    /// 補正後に残る書き込みクロックのジッタ (rms) [サンプル]
    pub residual_samples: f32,
    /// A/D 変換の量子化ビット数 (0 で量子化しない)
    pub bits: u32,
    /// 同期・ブランキングを作り直す (オフだと元の同期をそのまま通す)
    pub regenerate_sync: bool,
}

impl Default for TbcParams {
    fn default() -> Self {
        Self {
            enabled: false,
            window_samples: 30.0,
            gate_samples: 8.0,
            reacquire_lines: 3,
            error_rate: 0.0,
            residual_samples: 0.03,
            bits: 8,
            regenerate_sync: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum YcSeparation {
    /// トラップ (ノッチ) フィルタ。ドットクロールとクロスカラーが出る
    #[default]
    Notch,
    /// ラインくし形フィルタ (NTSC は 1H、PAL は 2H)
    Comb,
    /// 3 次元 (フレームくし形) Y/C 分離。画面バッファに持った前のフレーム
    /// (副搬送波が 180° ずれた NTSC 1 フレーム / PAL 2 フレーム前) と比べて分離する。
    /// 動きのある部分はラインくし形に切り替える (動き適応)
    #[serde(rename = "comb3d")]
    Comb3d,
}

/// 受像機 (NTSC / PAL デコーダ)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DecoderParams {
    pub yc_separation: YcSeparation,
    /// 3 次元 Y/C 分離の動き検出のしきい値 [IRE]。フレーム間差 (色副搬送波を除いたもの) がこれを超えると
    /// ラインくし形へ移り始め、2 倍で完全に切り替わる。大きいほど動きに鈍く、動く部分にクロスカラーの残像が出る
    pub motion_threshold_ire: f32,
    /// 3 次元 Y/C 分離の動き適応 (動き検出でラインくし形へ切り替える)。切ると常にフレームくし形になり、
    /// 動く部分の輪郭に前のフレームの絵が色の縞・二重像として残る
    pub motion_adaptive: bool,
    /// 3 次元 Y/C 分離の画面バッファ (フレームメモリ) のビット数。同期先端〜ピークを量子化する。
    /// 少ないと静止部分でもフレーム間差に量子化誤差が出て、色と輝度に細かい雑音・等高線が乗る。0 で量子化しない
    pub frame_buffer_bits: u32,
    /// 復調後の色差帯域 [MHz]
    pub chroma_bw_mhz: f32,
    /// 自動色レベル調整 (バースト振幅で正規化)
    pub acc: bool,
    /// バーストが弱いときに色を消す
    pub color_killer: bool,
    /// 水平 AFC の追従係数 (0 = 追従しない, 1 = 即時追従)
    pub h_pll: f32,
    /// 色副搬送波 PLL のバーストへの追従係数 (ライン単位。1 = ラインごとにバーストへ即時同期)。
    /// 小さいとテープのライン単位の位相ゆらぎを取り切れず、NTSC は色相の揺れ、PAL は彩度低下
    /// (遅延線なしならハノーバーバー) になる。実機の APC はループ帯域が数百 Hz で、0.1〜0.3 程度に当たる
    pub burst_pll: f32,
    /// 受像機の色副搬送波発振器 (VCXO) の自走周波数のずれ [Hz]。PLL の追従が有限 (`burst_pll` < 1) だと
    /// 定常位相誤差 ≒ 2π·Δf·T_H·(1 − k)/k が残り、NTSC では画面全体の色相ずれ、PAL では彩度低下になる。
    /// バーストのないライン (垂直帰線区間・カラーキラー域) では位相がこの速さで流れる
    pub vcxo_offset_hz: f32,
    /// PAL: 1H 遅延線で前のラインと色差を平均する (位相誤差が色相ずれではなく彩度低下になる)。
    /// 切ると「シンプル PAL」になり、位相誤差がハノーバーバー (ライン交互の色相ずれ) として見える
    pub pal_delay_line: bool,
    /// VHS HQ playback YNR strength (0 = bypass, 1 = strongest limited setting).
    pub ynr_strength: f32,
    /// Frame-cyclic playback DNR strength (0 = bypass, 1 = strongest setting).
    pub dnr_strength: f32,
    /// Motion threshold for the digital frame-cyclic DNR [IRE].
    pub dnr_motion_threshold_ire: f32,
    /// ADC/DAC resolution around playback NR (0 = ideal / no quantization).
    pub nr_bits: u32,
    pub contrast: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub hue_deg: f32,
    /// 垂直ホールド: 垂直発振器の自走周波数のずれ [%]。負 (自走が遅い) なら垂直同期で引き込まれる。
    /// 正や大きく負にずらすと同期を取れず、画面が上下に流れる (帰線区間の黒帯が見える)
    pub v_hold_pct: f32,
    /// 垂直同期を受け付ける範囲 (自走周期の終わりからの割合)。広いほど引き込みやすいが雑音で誤トリガしやすい
    pub v_sync_window: f32,
    /// 垂直同期分離の積分回路の時定数 [µs]
    pub v_integrator_us: f32,
    /// 画面の上下位置 [ライン]。負にすると帰線区間 (VBI のデータ・試験信号) が画面の上に見えてくる
    pub vertical_position: f32,
    /// 字幕デコーダ: ライン 21 / 284 のクローズドキャプションを解読して画面に出す (NTSC)
    pub caption_display: bool,
    /// 字幕デコーダで表示するチャンネル (1〜4 = CC1〜CC4)
    pub caption_channel: u8,
}

impl Default for DecoderParams {
    fn default() -> Self {
        Self {
            yc_separation: YcSeparation::Notch,
            motion_threshold_ire: 3.0,
            motion_adaptive: true,
            frame_buffer_bits: 8,
            chroma_bw_mhz: 1.3,
            acc: true,
            color_killer: true,
            h_pll: 0.25,
            burst_pll: 0.2,
            vcxo_offset_hz: 50.0,
            pal_delay_line: true,
            ynr_strength: 0.0,
            dnr_strength: 0.0,
            dnr_motion_threshold_ire: 6.0,
            nr_bits: 6,
            contrast: 1.0,
            brightness: 0.0,
            saturation: 1.0,
            hue_deg: 0.0,
            v_hold_pct: -4.0,
            v_sync_window: 0.2,
            v_integrator_us: 30.0,
            vertical_position: 0.0,
            caption_display: false,
            caption_channel: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Deinterlace {
    /// 2 フィールドを交互に織り込む (インターレースのまま)
    #[default]
    Weave,
    /// 現フィールドのラインを 2 倍に引き伸ばす
    Bob,
    /// 隣接するフィールドラインを平均して補間
    Blend,
    /// 静止部は前フィールドを使い、動き部は空間補間
    MotionAdaptiveBlend,
    /// 時間方向の動き検出とエッジ方向の空間補間
    Yadif,
    /// 動き補償と時間・空間補間を組み合わせる高品質モード
    Qtgmc,
}

/// 再生機 → 受像機の接続
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    /// コンポジット (映像端子 1 本)。受像機で Y/C 分離するのでドットクロール・クロスカラーが出る
    #[default]
    Composite,
    /// S 映像 (Y/C 分離のまま)。テープを通すときだけ有効 (放送受信・RF 接続はコンポジット)
    SVideo,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OutputParams {
    pub deinterlace: Deinterlace,
    pub deinterlace_params: DeinterlaceParams,
    /// 再生機 → 受像機の接続
    pub connection: Connection,
}

/// 方式別のインターレース解除調整値。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeinterlaceParams {
    /// Weave の前フィールド保持率 [%]
    pub weave_temporal_weight_pct: f32,
    /// Bob の隣接ライン補間量 [%]
    pub bob_blend_pct: f32,
    /// Blending の隣接ライン平均量 [%]
    pub blend_strength_pct: f32,
    /// 動き適応 Blending の動き判定しきい値 (8-bit 輝度差)
    pub motion_adaptive_threshold: f32,
    /// 動き適応 Blending の動き部分の隣接ライン平均量 [%]
    pub motion_adaptive_blend_pct: f32,
    /// Yadif の動き判定しきい値 (8-bit 輝度差)
    pub yadif_motion_threshold: f32,
    /// Yadif の水平方向のエッジ探索半径 [px]
    pub yadif_search_radius: u32,
    /// Yadif の静止部における時間方向の混合量 [%]
    pub yadif_temporal_weight_pct: f32,
    /// QTGMC 風実装の動き探索ブロック幅 [px] (高さは半分)
    pub qtgmc_block_size: u32,
    /// QTGMC 風実装の動き探索半径 [px / field line]
    pub qtgmc_search_radius: u32,
    /// QTGMC 風実装の動き補償フレーム混合量 [%]
    pub qtgmc_temporal_weight_pct: f32,
    /// QTGMC 風実装のエッジ強調量 [%]
    pub qtgmc_sharpness_pct: f32,
}

impl Default for DeinterlaceParams {
    fn default() -> Self {
        Self {
            weave_temporal_weight_pct: 100.0,
            bob_blend_pct: 0.0,
            blend_strength_pct: 100.0,
            motion_adaptive_threshold: 10.0,
            motion_adaptive_blend_pct: 100.0,
            yadif_motion_threshold: 10.0,
            yadif_search_radius: 3,
            yadif_temporal_weight_pct: 100.0,
            qtgmc_block_size: 16,
            qtgmc_search_radius: 2,
            qtgmc_temporal_weight_pct: 88.0,
            qtgmc_sharpness_pct: 12.0,
        }
    }
}

/// ブラウン管シミュレーション。RetroArch の slang シェーダープリセット (.slangp) を
/// 最終出力に掛ける (crt-royale, crt-guest-advanced など)。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CrtParams {
    pub enabled: bool,
    /// プリセット (.slangp) のパス
    pub preset: String,
    /// 出力の高さ [px]。幅は 4:3 で決める (正方画素)
    pub height: u32,
    /// シェーダーパラメータの上書き (プリセットの既定値から変えたものだけ)
    pub params: std::collections::BTreeMap<String, f32>,
}

impl Default for CrtParams {
    fn default() -> Self {
        Self { enabled: false, preset: String::new(), height: 1080, params: Default::default() }
    }
}

/// 静止画を画面にどう合わせるか
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    /// 縦横比を無視して画面いっぱいに引き伸ばす
    #[default]
    Stretch,
    /// 元画像の画素数を保つ
    Original,
    /// 縦横比を保って画面を覆う (はみ出した部分はパンで見る)
    Cover,
    /// 縦横比を保って画面に収める (余白は黒)
    Contain,
    /// ドットクロック同期: 元画像の 1 画素を 4fsc の整数サンプル (横) × 整数ライン (縦) にそのまま置く
    /// (補間しない)。倍率は画面に収まる最大の整数 (640 → 1 サンプル、320 → 2 サンプル、200 → 2 ライン)。
    /// 横位置は副搬送波の 1 周期 (4 サンプル) 単位に揃えるので、画素の縞が副搬送波と同期して
    /// アーティファクトカラーになる。拡大率は使わない
    Pixel,
}

/// 静止画入力の配置とパン (カメラを振る動き)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ImageParams {
    pub fit: ImageFit,
    /// 拡大率 (1 = `fit` の大きさ)
    pub zoom: f32,
    /// 表示位置 (-1 = 左端/上端, 0 = 中央, 1 = 右端/下端)
    pub pan_x: f32,
    pub pan_y: f32,
    /// パンの速さ [画面幅/秒], [画面高さ/秒]。0 なら `pan_x`/`pan_y` の位置で止まる。端で折り返す
    pub speed_x: f32,
    pub speed_y: f32,
    /// HDR 画像 (OpenEXR) の露出 [EV]
    pub exposure_ev: f32,
    /// ドットクロック同期の配置で、画像を右へずらすサンプル数 (0..3)。副搬送波との位相が 90° ずつ変わり、
    /// アーティファクトカラーの色相が変わる
    pub pixel_phase: u8,
}

impl Default for ImageParams {
    fn default() -> Self {
        Self { fit: ImageFit::Stretch, zoom: 1.0, pan_x: 0.0, pan_y: 0.0, speed_x: 0.0, speed_y: 0.0, exposure_ev: 0.0, pixel_phase: 0 }
    }
}

/// OSD の合成先
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OsdMode {
    /// アナログ映像処理を通さず、最終画面に直接重ねる
    Direct,
    /// キャプチャ処理の直前に重ねる
    Capture,
    /// テープ再生後のコンポジット信号にデッキが重畳する
    #[default]
    Composite,
    /// 録画時にテープへ焼き込まれている (カメラの日付表示など)
    Tape,
}

/// OSD (オンスクリーン表示)。書式は `osd::markup` を参照。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OsdParams {
    pub enabled: bool,
    pub mode: OsdMode,
    /// 表示内容のマークアップ
    pub markup: String,
    /// 既定の文字サイズ (1 = 2×2 ドット/画素, 1〜8)
    pub scale: u32,
    /// 不透明度 (0..1)
    pub opacity: f32,
    /// `{date}` `{time}` などの起点となる日時 ("YYYY-MM-DD HH:MM:SS")
    pub start_datetime: String,
    /// フォントファイル (.ttf / .otf / .ttc) のパス。空なら内蔵の VCR-JP フォント
    pub font: String,
    /// .ttc 内のフォント番号
    pub font_index: u32,
    /// 外部フォントを 1 em あたり何ドットで表示するか (0 = なめらかに描く)
    pub font_dots: u32,
    /// ドット表示時に SAA5050 風の斜め補間をする
    pub font_rounding: bool,
}

impl Default for OsdParams {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: OsdMode::Composite,
            markup: "<pos=56,48><edge=black>PLAY ▶</edge>
<pos=56,400><edge=black>{counter}   <color=#9cf>SP</color></edge>
<pos=698,400><align=right><edge=black>{MON}.{D} {Y}</edge>".into(),
            scale: 2,
            opacity: 1.0,
            start_datetime: "1995-08-12 14:30:00".into(),
            font: String::new(),
            font_index: 0,
            font_dots: 16,
            font_rounding: true,
        }
    }
}

/// RF 接続 (チューナ経由) をパイプラインのどこに入れるか
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RfPlacement {
    /// テープの前: 放送を受信して録画した (受信時の劣化ごとテープに記録される)
    BeforeTape,
    /// テープの後: デッキの RF 出力をテレビのアンテナ端子で受けた
    #[default]
    AfterTape,
    /// 両方: 放送を録画し、さらにデッキから RF でテレビに入れた (雑音は独立)
    Both,
}

impl RfPlacement {
    pub fn before_tape(self) -> bool {
        matches!(self, Self::BeforeTape | Self::Both)
    }

    pub fn after_tape(self) -> bool {
        matches!(self, Self::AfterTape | Self::Both)
    }
}

/// RF 接続 (VCR の RF モジュレータ → テレビ、または放送受信) のチューナ経由の劣化
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RfParams {
    pub enabled: bool,
    /// どこで RF を通すか (放送受信 / デッキ→テレビ / 両方)
    pub placement: RfPlacement,
    /// 受信ノイズ (スノー) の rms [IRE]
    pub snow_ire: f32,
    /// ゴースト (反射波) の遅延 [µs]
    pub ghost_delay_us: f32,
    /// ゴーストの強さ (負なら反転ゴースト)
    pub ghost_gain: f32,
    /// ビート妨害 (斜め縞) の振幅 [IRE]
    pub interference_ire: f32,
    /// ビート妨害の周波数 [MHz]
    pub interference_mhz: f32,
    /// 音声搬送波が映像に漏れる量 (920 kHz ビート / 音声連動の縞)
    pub sound_beat: f32,
}

impl Default for RfParams {
    fn default() -> Self {
        Self {
            enabled: false,
            placement: RfPlacement::AfterTape,
            snow_ire: 3.0,
            ghost_delay_us: 1.2,
            ghost_gain: 0.12,
            interference_ire: 0.0,
            interference_mhz: 2.7,
            sound_beat: 0.0,
        }
    }
}

/// キャプチャ時のレベル (ビデオレンジ/フルレンジ) の扱い
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureLevels {
    /// 正しく変換
    #[default]
    Correct,
    /// 16-235 をフルレンジとして表示 (黒が浮いて白がくすむ)
    WashedOut,
    /// フルレンジを 16-235 として表示 (黒つぶれ・白飛び)
    Crushed,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChromaSubsampling {
    #[serde(rename = "444")]
    S444,
    #[serde(rename = "422")]
    S422,
    /// MPEG-2 / DVD / 多くの USB キャプチャ
    #[default]
    #[serde(rename = "420")]
    S420,
    /// NTSC DV
    #[serde(rename = "411")]
    S411,
}

/// デジタルキャプチャ (アナログキャプチャボード → MPEG 系エンコーダ)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CaptureParams {
    pub enabled: bool,
    pub levels: CaptureLevels,
    pub chroma_subsampling: ChromaSubsampling,
    /// DCT ブロック圧縮 (MPEG-2 風) を行う
    pub codec: bool,
    /// 目標ビットレート [kbps] (29.97 fps 換算)
    pub bitrate_kbps: f32,
    /// GOP 長 (I フレーム間隔) [フレーム]
    pub gop: u32,
    /// 動き探索範囲 [画素] (0 で動き補償なし)
    pub motion_search: u32,
    /// レート制御の追従の速さ (0..1)。大きいほど画質が「呼吸」する
    pub rate_control: f32,
    /// コマ落ち (前フレームの繰り返し) の確率
    pub frame_drop_rate: f32,
    /// ADC の量子化ビット数 (8 が普通)
    pub adc_bits: u32,
}

impl Default for CaptureParams {
    fn default() -> Self {
        Self {
            enabled: false,
            levels: CaptureLevels::Correct,
            chroma_subsampling: ChromaSubsampling::S420,
            codec: true,
            bitrate_kbps: 4000.0,
            gop: 15,
            motion_search: 7,
            rate_control: 0.5,
            frame_drop_rate: 0.0,
            adc_bits: 8,
        }
    }
}

/// 音声トラックの方式
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioMode {
    /// 固定ヘッドによるリニア (ノーマル) 音声トラック: モノラル・狭帯域・ヒス・ワウフラッタ
    #[default]
    Linear,
    /// 回転ヘッドによる Hi-Fi (FM 深層記録) 音声: ステレオ・広帯域だがヘッド切替バズが出る
    Hifi,
}

/// 音声の劣化 (テープ → 再生機 → キャプチャ)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioParams {
    pub enabled: bool,
    pub mode: AudioMode,

    // --- テープ ---
    /// リニアトラックの低域カットオフ [Hz]
    pub low_cut_hz: f32,
    /// リニアトラックの録音・再生アンプの高域カットオフ [Hz]。速度・スペーシングによるヘッドの損失は別に掛かる
    pub high_cut_hz: f32,
    /// リニアトラックのテープ速度 (SP に対する比。LP = 0.5、EP = 1/3)。
    /// 再生 EQ は SP 用のままなので、遅くするとスペーシング・ギャップの損失で高域が落ちる
    pub tape_speed: f32,
    /// 音声ヘッドのギャップ長 [µm]。波長がこれと等しくなる周波数 (= 速度 / ギャップ) でヌルになる
    pub head_gap_um: f32,
    /// 音声ヘッドとテープの余分な間隔 [µm] (ヘッドの汚れ・テープの摩耗)
    pub head_spacing_um: f32,
    /// 交流バイアスの記録磁界 [保磁力 Hc の倍数] (磁性体モデルが J-A のとき)。
    /// 2 前後でほぼ直線、少ないと小音量がデッドゾーンで歪み大音量で早く飽和、多いと感度が落ちる
    pub bias_level: f32,
    /// テープヒス [dBFS]
    pub hiss_db: f32,
    /// 録音レベル (テープ飽和へのドライブ) [dB]
    pub drive_db: f32,
    /// ワウ (遅い速度変動) [%]
    pub wow_pct: f32,
    /// フラッタ (速い速度変動) [%]
    pub flutter_pct: f32,
    /// ドロップアウト (音がこもって痩せる) [回/秒]
    pub dropout_rate: f32,
    /// Hi-Fi のヘッド切替バズ (トラッキングずれでも増える)
    pub hifi_buzz: f32,

    // --- 周辺機器 ---
    /// 電源ハム (グラウンドループ) [dBFS]
    pub hum_db: f32,
    /// 電源周波数 [Hz]
    pub hum_hz: f32,
    /// AGC (自動録音レベル) の強さ (0..1)。ポンピングと無音部のノイズ持ち上げ
    pub agc: f32,
    /// キャプチャ ADC のサンプリング周波数 [Hz]
    pub sample_rate_hz: f32,
    /// キャプチャ ADC の量子化ビット数
    pub bits: u32,
    /// モノラルにする
    pub mono: bool,
    /// 書き出し時の AAC ビットレート [kbps] (低いほど圧縮ノイズ)
    pub codec_kbps: u32,
}

impl Default for AudioParams {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: AudioMode::Linear,
            low_cut_hz: 80.0,
            high_cut_hz: 10000.0,
            tape_speed: 1.0,
            head_gap_um: 1.0,
            head_spacing_um: 0.0,
            bias_level: 2.0,
            hiss_db: -52.0,
            drive_db: 0.0,
            wow_pct: 0.12,
            flutter_pct: 0.08,
            dropout_rate: 0.05,
            hifi_buzz: 0.1,
            hum_db: -90.0,
            hum_hz: 60.0,
            agc: 0.0,
            sample_rate_hz: 48000.0,
            bits: 16,
            mono: false,
            codec_kbps: 192,
        }
    }
}
