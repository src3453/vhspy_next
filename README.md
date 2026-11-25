# VHS Next

磁気テープ (VHS) の記録・再生をシミュレートして映像を劣化させるツール。
旧 Python 版 (`vhs_next.py`) を Rust + Tauri で書き直したもの。

NTSC-CRT などの既存プロジェクトは主に NTSC のエンコード/デコードと CRT 表示を扱う。
このプロジェクトは、その間に入る **磁気テープの記録・再生経路** を物理的にモデル化することに重点を置く。

## 構成

```
crates/vhs-core   信号処理ライブラリ (GUI 非依存)
src-tauri         Tauri バックエンド (入力・処理スレッド・録画)
src               Svelte フロントエンド (プレビュー・パラメータ UI)
vhs_next.py       旧実装 (参照用)
```

### 信号の流れ

```
RGB 画像 ─ encoder ─▶ コンポジット信号 (4fsc, 910 サンプル/ライン, IRE)
         ─ rf ──────▶ 放送受信 (録画前の RF)                           ※任意
         ─ tape ────▶ 記録/再生経路 → 媒体欠陥 → 走行系 (時間軸誤差)
         ─ rf ──────▶ RF 接続 (スノー, ゴースト, ビート妨害)            ※任意
                      (rf.placement: after_tape / before_tape / both)
         ─ decoder ─▶ 受像機 (水平 AFC, Y/C 分離, バースト同期検波) ─▶ RGBA
         ─ capture ─▶ ADC → 色差間引き → DCT 圧縮 (MPEG-2 風)       ※任意 ─▶ RGBA

音声 (書き出し時) ─ audio ─▶ テープ音声トラック → 再生機・キャプチャ機器 ─▶ AAC
```

- 1 入力フレーム = 1 フィールド (59.94 Hz)。出力は 754×480 (4:3 で表示)
- 信号はすべて 1 次元のコンポジット信号として扱う。ドットクロールやクロスカラーは
  画素単位のエフェクトではなく、デコーダの Y/C 分離の結果として現れる
- 時間軸誤差は同期信号とバーストを含むライン全体をずらす。補正は受像機側の AFC と
  バースト位相同期に任せるため、遅い変動は吸収され、速い変動は残る

### テープモデル (`crates/vhs-core/src/tape/`)

| 経路 | 現在のモデル |
| --- | --- |
| 輝度 (FM) | Y/C 分離 → 帯域制限 → エンファシス → ホワイトクリップ → FM 記録 (磁性体は tanh または JA PoC) → 再生帯域・ノイズ → 復調 |
| 色 (低域変換) | 複素包絡線で等価的に処理: 狭帯域化 → 磁気飽和 → AM/位相ノイズ → Y/C 遅延 |
| 磁性体 | tanh / 線形。Jiles–Atherton は FM 輝度 RF のみ、ラインごとに初期化して 4 倍処理する PoC |
| 媒体 | ドロップアウト (DOC あり/なし)、しわ (流れる帯状ノイズ)、ヘッド目詰まり (一瞬の砂嵐) |
| 走行系 | ジッタ、ワウフラッタ、スキュー、ヘッド切替、トラッキングずれ (ノイズバー)、垂直揺れ |

### 再生・視聴環境のモデル

NTSC の忠実さだけでなく「実際に再生して取り込んだらどう見えるか / 聞こえるか」を足す段。

| 段 | モデル |
| --- | --- |
| RF 接続 (`rf.rs`) | コンポジットのまま: チューナ帯域の白色雑音 (同期にも乗る)、反射波ゴースト、ビート妨害、音声搬送波との 920 kHz ビート |
| キャプチャ (`capture/`) | BT.601 変換とレベル取り違え (黒浮き/黒つぶれ)、ADC ビット数、4:2:0 / 4:1:1 / 4:2:2 間引き (インターレース素材での色にじみ込み)、8×8 DCT + MPEG-2 量子化マトリクス、動き補償 P フレーム (閉ループ)、GOP、符号量見積もりによるレート制御 (ノイズが符号量を食い画質が呼吸する)、コマ落ち |
| 音声 (`audio.rs`) | リニア: モノラル・帯域制限・ヘッドバンプ・磁気飽和・ワウフラッタ (変調遅延線)・ヒス・ドロップアウト。Hi-Fi: ステレオ・コンパンダによるノイズの浮き沈み・59.94 Hz のヘッド切替バズ (トラッキングずれで悪化)。周辺機器: 電源ハム、AGC のポンピング、ADC のサンプリング周波数・ビット数、AAC ビットレート |

受像機の同期分離は実機同様に積分してからスライスするので、スノーが多少あっても水平同期は保たれる。

垂直同期のエミュレーションは未完成。現在の `Field` は有効映像ラインだけを保持し、垂直帰線区間や
垂直同期パルス列 (等化パルス・セレーションを含む) を信号として表現していない。「信号劣化」の
垂直同期ジッタは画像の上下移動として、レベル減衰と雑音は簡易なロック外れ判定として近似する。
垂直同期波形や実機の垂直ロック動作を再現するものではない。

### インターレース解除

出力設定から Weave、Bob、Blending、Motion-adaptive Blending、Yadif (Yadifmod2)、QTGMC を選べる。
GUI の「出力 (インターレース解除)」には方式別の調整項目があり、Weave/Bob/Blending は前フィールド保持量や隣接ライン補間量、動き適応方式は動きしきい値・エッジ探索・時間合成、QTGMC風実装はブロックサイズ・探索半径・時間合成・シャープネスを調整できる。
QTGMC は **簡易版・未完成**。現状はブロック単位の動き補償とエッジ方向補間を組み合わせた試作で、
標準 QTGMC の全工程、高精度の多段動き推定、時間ノイズ除去、ソースマッチングは実装していない。
標準 QTGMC と同等の画質や動作を保証するものではない。

## 実行

必要なもの: Rust, Node.js, ffmpeg (PATH 上、または環境変数 `VHS_FFMPEG`)

```powershell
npm install
npm run tauri dev
```

入力はテストパターン、カメラ (DirectShow。OBS 仮想カメラも可)、動画、静止画。
録画とオフライン書き出し (音声付き) は ffmpeg (libx264) で行う。

### イマーシブモード

ツールバーの「イマーシブ」か F11 で、UI を隠して映像だけを全画面に出し、デッキのメニュー表示で操作する。
メニューはビデオデッキのキャラクタジェネレータのように再生出力 (コンポジット / S 映像) に重畳するので、
RF・受像機・CRT シェーダーの劣化を映像と一緒に受ける (録画にも映る)。文字は内蔵の VCR-JP フォント
(`crates/vhs-core/fonts/vcr-jp.ttf`、半角カナ対応) を SAA5050 風に斜めを補間して描く。

| キー | メニューなし | メニュー中 |
| --- | --- | --- |
| Enter / M | メニューを開く | 決定 (M は閉じる) |
| ↑ ↓ | トラッキング | 項目の選択 |
| ← → | | 値の変更 (Shift で 10 倍) |
| Esc / Backspace | 終了 (Esc) | 戻る |
| Space | 入力・カウンタを表示 | |
| F11 | 終了 | 終了 |

マウスはクリックでメニューの開閉、ホイールで選択。範囲のある値は Enter で既定値に戻る。

### 開発用

```powershell
cargo test -p vhs-core --release
cargo run -p vhs-core --release --example render -- --bench
cargo run -p vhs-core --release --example render -- input.png out.png [params.json] [fields] [pan_px]
# デッキのメニュー表示を重ねて確かめる (OSD マークアップ)
$env:VHS_MENU = '<pos=40,40><bg=black>ﾒﾆｭｰ MENU</bg>'; cargo run -p vhs-core --release --example render -- --pattern out.png
# 音声だけ: f32le / 48 kHz / ステレオを標準入出力で
ffmpeg -i in.mp4 -vn -ac 2 -ar 48000 -f f32le - | cargo run -p vhs-core --release --example audio -- [params.json] | ffmpeg -f f32le -ar 48000 -ac 2 -i - out.wav
```

### レンダーパス連携

Blenderなどのホストから使うためのC ABI (`vhs-ffi`) と、Render Resultを処理する
Blenderアドオン、Unity URP/HDRP向けパッケージを追加。シーンリニアな `float32 RGBA` を受け取り、フレーム間の
テープ状態を保持したまま処理する。Unity版は非同期GPU読み戻しを使い、数フレーム遅れて出力する。
導入手順は [docs/render-hosts.md](docs/render-hosts.md) と [integrations/unity/README.md](integrations/unity/README.md) を参照。

## 今後

- 輝度 FM の実変調/復調 (オーバーサンプリングして、FM の側帯波とモアレを再現する)
- 減磁、プリントスルー
- アジマス記録とクロストーク、VHS 色位相回転とくし形フィルタによる打ち消し
- 垂直帰線区間と垂直同期パルス列の明示的なモデル化、垂直ロック動作の詳細化
- 磁性体モデル: [J. Chowdhury, Real-Time Physical Modelling for Analog Tape Machines (DAFx-19)](https://dafx.de/paper-archive/2019/DAFx2019_paper_3.pdf)
- その他の参考資料: [AV Artifact Atlas](https://www.avartifactatlas.com/)
