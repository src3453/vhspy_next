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
         ─ tape ────▶ 記録/再生経路 → 媒体欠陥 → 走行系 (時間軸誤差)
         ─ decoder ─▶ 受像機 (水平 AFC, Y/C 分離, バースト同期検波) ─▶ RGBA
```

- 1 入力フレーム = 1 フィールド (59.94 Hz)。出力は 754×480 (4:3 で表示)
- 信号はすべて 1 次元のコンポジット信号として扱う。ドットクロールやクロスカラーは
  画素単位のエフェクトではなく、デコーダの Y/C 分離の結果として現れる
- 時間軸誤差は同期信号とバーストを含むライン全体をずらす。補正は受像機側の AFC と
  バースト位相同期に任せるため、遅い変動は吸収され、速い変動は残る

### テープモデル (`crates/vhs-core/src/tape/`)

| 経路 | 現在のモデル |
| --- | --- |
| 輝度 (FM) | Y/C 分離 → 帯域制限 → エンファシス → ホワイトクリップ → FM ノイズ・過変調 (ベアディング) → ディエンファシス |
| 色 (低域変換) | 複素包絡線で等価的に処理: 狭帯域化 → 磁気飽和 → AM/位相ノイズ → Y/C 遅延 |
| 媒体 | ドロップアウト (DOC あり/なし) |
| 走行系 | ジッタ、ワウフラッタ、スキュー、ヘッド切替、トラッキングずれ (ノイズバー) |

## 実行

必要なもの: Rust, Node.js, ffmpeg (PATH 上、または環境変数 `VHS_FFMPEG`)

```powershell
npm install
npm run tauri dev
```

入力はテストパターン、カメラ (DirectShow。OBS 仮想カメラも可)、動画、静止画。
録画とオフライン書き出し (音声付き) は ffmpeg (libx264) で行う。

### 開発用

```powershell
cargo test -p vhs-core --release
cargo run -p vhs-core --release --example render -- --bench
cargo run -p vhs-core --release --example render -- input.png out.png [params.json]
```

## 今後

- 輝度 FM の実変調/復調 (オーバーサンプリングして、FM の側帯波とモアレを再現する)
- 磁性体のヒステリシスモデル (Jiles–Atherton)、減磁、プリントスルー
- アジマス記録とクロストーク、VHS 色位相回転とくし形フィルタによる打ち消し
- 同期分離・垂直同期の乱れ、TBC の有無
- 参考資料: [AV Artifact Atlas](https://www.avartifactatlas.com/)
