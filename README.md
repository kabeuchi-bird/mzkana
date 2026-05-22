# MzKana

Fcitx5 上で動作する、かな配列・漢直入力エンジンです。  
変換には Mozc を流用し、入力ロジックのみを純 Rust で実装します。

## 特徴

- **配列不問** — TOML 設定ファイルを差し替えるだけで任意のかな配列を使用できます
- **6 つのシフト方式に対応** — 通常シフト / 同時シフト / 前置シフト / 後置シフト / 相互シフト / センターシフト
- **漢直サポート** — T-code / TUT-code など 2 ストローク漢字直接入力に対応
- **投機的送信** — 入力をリアルタイムに Mozc へ送り、後から確定・書き換え（BS-rewrite）
- **Mozc IPC クライアント** — Unix ドメインソケット経由で mozc_server に直接接続
- **機能キー出力** — `!Return` / `!Tab` など機能キーをグリッドや chord に埋め込み可能
- **エイリアス / 複数トークン出力** — `[[alias]]` で名前付きシーケンスを定義し、グリッドや chord から参照
- **Fcitx5 アドオン** — C++ シムレイヤー経由でインライン preedit・コミット・ホットリロードに対応

## リポジトリ構成

```
mzkana/
├── crates/
│   ├── mzkana-core/        # 状態機械・設定パーサ・Mozc IPCクライアント（ライブラリ）
│   │   └── src/mozc/       # protobufコーデック + UDSクライアント
│   ├── mzkana-cli/         # 検証・デバッグ用 CLI ツール
│   └── mzkana-ffi/         # C ABI ラッパー（cbindgen で mzkana.h を生成）
├── fcitx5-addon/           # Fcitx5 C++ アドオン
│   ├── src/engine.cpp/.h   # InputMethodEngineV2 実装
│   ├── data/               # mzkana.conf（アドオン登録）/ mzkana-im.conf（入力メソッド登録）
│   └── cmake/include/      # スタブエクスポートヘッダ（-dev 不要ビルド用）
├── layouts/                # サンプル配列定義
│   ├── tsuki-2-263.toml    # 月配列 2-263 式（前置シフト）
│   ├── shin-geta.toml      # 新下駄配列（同時シフト）
│   ├── naginata-v17.toml   # 薙刀式 v17（センターシフト + 相互シフト）
│   └── t-code.toml         # T-code（漢直）
└── schemas/                # JSON Schema（エディタ補完用）
```

## ビルド

### Rust ライブラリ・CLI

```sh
cargo build
```

Mozc のインストールは不要でビルドできます。  
`mozc-run` サブコマンドの実行時のみ `mozc_server` の起動が必要です。

### Fcitx5 アドオン

アドオンのビルドには **Rust リリースビルド**（`libmzkana.so`）と fcitx5 ヘッダが必要です。

#### A. `libfcitx5-core-dev` がインストール済みの場合（推奨）

```sh
sudo apt install libfcitx5-core-dev cmake

cargo build --release -p mzkana-ffi

cmake -B build fcitx5-addon
cmake --build build
sudo cmake --install build
```

#### B. ランタイムパッケージのみの場合（-dev 不要）

```sh
# fcitx5 ソースツリーからヘッダを取得（sparse checkout）
git clone --depth=1 --filter=blob:none --sparse \
    https://github.com/fcitx/fcitx5.git /tmp/fcitx5-src
git -C /tmp/fcitx5-src sparse-checkout set src/lib

# unversioned シンボリックリンクをビルドディレクトリ内に作成（sudo 不要・システム変更なし）
mkdir -p build/fcitx5-link
ln -sf "$(find /usr/lib -name 'libFcitx5Core.so.*' | grep -v '\.so\.[0-9]*\.' | head -1)" \
       build/fcitx5-link/libFcitx5Core.so
ln -sf "$(find /usr/lib -name 'libFcitx5Utils.so.*' | grep -v '\.so\.[0-9]*\.' | head -1)" \
       build/fcitx5-link/libFcitx5Utils.so

cargo build --release -p mzkana-ffi

cmake -B build fcitx5-addon \
  -DFCITX5_USE_SRC_HEADERS=ON \
  -DFCITX5_CORE_LIB="$PWD/build/fcitx5-link/libFcitx5Core.so" \
  -DFCITX5_UTILS_LIB="$PWD/build/fcitx5-link/libFcitx5Utils.so"
cmake --build build
sudo cmake --install build
```

#### アドオン設定ファイルの配置

```sh
mkdir -p ~/.config/fcitx5/conf/mzkana
cp layouts/naginata-v17.toml ~/.config/fcitx5/conf/mzkana/layout.toml
```

配置後、fcitx5 を再起動するか `fcitx5-remote -r` で再ロードしてください。

#### ホットリロード

`~/.config/fcitx5/conf/mzkana/layout.toml` を編集・保存すると、次のキーイベント時に自動的に再ロードされます（fcitx5 の再起動不要）。

## fcitx5-configtool での設定

インストール後、以下の手順で MzKana を有効にします。

### 1. MzKana を入力メソッドリストに追加する

```sh
fcitx5-configtool
```

1. **「入力メソッド」タブ** を開く
2. 右下の **「入力メソッドを追加」（`+`）** ボタンをクリック
3. 検索ボックスに `mzkana` または `MzKana` と入力
4. **「MzKana」** を選択して **「OK」**

### 2. 入力メソッドを切り替える

| 操作 | 方法 |
|---|---|
| MzKana へ切り替え | `Ctrl+Space`（デフォルト）または fcitx5 のトレイアイコンをクリック |
| 前の入力メソッドへ戻る | 同キーをもう一度押すか、トレイから選択 |

切り替えキーは fcitx5-configtool の **「グローバルオプション」→「ホットキー」** から変更できます。

### 3. レイアウトファイルを選ぶ

レイアウトファイルの配置方法と利用可能な配列については、上記の「アドオン設定ファイルの配置」と「ホットリロード」の項を参照してください。

### 4. アドオンの有効・無効を切り替える

```sh
fcitx5-configtool
```

**「アドオン」タブ** → `MzKana` の行でチェックボックスをオン／オフすることで、アドオン自体を有効化・無効化できます。

## CLI ツール

```sh
# 配列ファイルの検証
cargo run -p mzkana-cli -- validate layouts/tsuki-2-263.toml

# キーシーケンスを流して出力アクションを確認
cargo run -p mzkana-cli -- run layouts/tsuki-2-263.toml --keys "d w"

# 同時押し（+）でコードを表現
cargo run -p mzkana-cli -- run layouts/shin-geta.toml --keys "f+j"

# キーアップを ^ で表現
cargo run -p mzkana-cli -- run layouts/naginata-v17.toml --keys "space q space^"

# Mozc サーバに接続してキーシーケンスを送り、preedit/result を確認
cargo run -p mzkana-cli -- mozc-run layouts/naginata-v17.toml --keys "k a s d"

# Mozc ソケットのパスを明示する場合
cargo run -p mzkana-cli -- mozc-run layouts/naginata-v17.toml \
    --socket ~/.mozc/server.sock --keys "k a s d"

# JSON Schema を出力（エディタの補完設定に利用）
cargo run -p mzkana-cli -- schema
```

### `run` の出力例

```text
send_kana(か)
send_kana(た)
```

### `mozc-run` の出力例（mozc_server 起動済みの場合）

```text
Connected to Mozc (session 1)
send_kana(か)
  → preedit: か
send_kana(た)
  → preedit: かた
```

## 配列ファイル形式

```toml
[meta]
name   = "配列名"
mode   = "kana"   # kana | kanchoku | hybrid
schema = 1

[settings]
chord_window_ms  = 50   # 同時打鍵の受付ウィンドウ（ms）
mutual_window_ms = 80   # 相互シフトの受付ウィンドウ（ms）

# ── ベースレイヤー ────────────────────────────────────────────────
[[layer]]
id   = "base"
kind = "single"
grid = """
. q    w    e    r    t    y    u    i    o    p
1 。   か   た   こ   さ   ら   ち   く   つ   、
2 う   し   て   け   せ   は   と   き   い   ん
3 ＿   に   な   っ   く   は   の   や   ゅ   を
"""

# ── 前置シフト ────────────────────────────────────────────────────
[[layer]]
id      = "prefix_d"
kind    = "prefix"
trigger = "d"
grid    = """
. q    w    e    r    t    y    u    i    o    p
1 ぁ   え   り   ゃ   れ   ぱ   ぢ   ぐ   づ   ぴ
2 を   ゔ   ぃ   ぬ   ょ   ふ   ご   げ   ぞ   ぼ
3 ぅ   ひ   そ   み   ＿   ゆ   に   ま   の   ＿
"""

# ── 同時打鍵（chord）────────────────────────────────────────────
[[chord]]
keys      = ["f", "j"]
output    = "を"
symmetric = true   # 押下順序を問わない

# ── センターシフト（modifier）────────────────────────────────────
[[modifier]]
id              = "center"
key             = "space"
kind            = "hold"
hold_detection  = "interrupt"
tap_action      = "send_key"

[[layer]]
id       = "center_shift"
kind     = "modified"
modifier = "center"
grid     = """
. q    w
1 ぁ   へ
"""

# ── エイリアス（名前付きシーケンス）──────────────────────────────
[[alias]]
ku_ret = "、 !Return"   # グリッドや chord の output から参照可能
to_ret = "。 !Return"

# ── 漢直ルール ────────────────────────────────────────────────────
[[direct]]
sequence = ["k", "j"]
output   = "日"
```

### グリッドの書き方

| 記法 | 意味 |
|---|---|
| `あ` | そのかな文字を送信 |
| `＿` | 空セル（割り当てなし） |
| `!Return` | 機能キー `Return` を送信 |
| `"、 !Return"` | 複数トークンを順番に送信（引用符で囲む） |
| `ku_ret` | `[[alias]]` で定義した名前を参照 |

**行ラベル**とキーの対応：

| 行ラベル | キー（左→右） |
|---|---|
| `0` | `1 2 3 4 5 6 7 8 9 0 minus equal yen` |
| `1` | `q w e r t y u i o p` |
| `2` | `a s d f g h j k l semicolon` |
| `3` | `z x c v b n m comma period slash` |

### 機能キー一覧（`!` プレフィックス）

`Return` / `Tab` / `Escape` / `BackSpace` / `Delete` /
`Home` / `End` / `Insert` / `Up` / `Down` / `Left` / `Right` /
`Prior` / `Next` / `PageUp` / `PageDown` /
`F1`–`F12` / `space` / `Henkan` / `Muhenkan` / `Hiragana_Katakana`

## Mozc IPC について

`mzkana-core` は Mozc の `commands.proto` を使って `mozc_server` と直接通信します。

- **接続先**: `~/.mozc/server.sock`（`--socket` で変更可）
- **プロトコル**: `uint32_le(メッセージ長) + protobuf バイト列`（双方向）
- **外部依存**: prost / protoc 不要（Mozc の proto2 group 型に対応した独自コーデックを内蔵）
- **セッション管理**: 接続時に `CREATE_SESSION`、切断時に `DELETE_SESSION` を自動実行

## Fcitx5 アドオンのアーキテクチャ

```text
キーイベント
    │
    ▼
fcitx5-mzkana.so  (C++, InputMethodEngineV2)
    │  key_name + shift フラグに正規化
    ▼
libmzkana.so  (Rust FFI, mzkana-ffi)
    │  MzkanaResult { consumed, preedit, commit, passthrough_key }
    ▼
MzkanaEngine  (mzkana-core)
    ├─ 状態機械（シフト / chord / 漢直）
    └─ Mozc IPC クライアント（preedit 同期・投機的送信）
```

- `mzkana_engine_key_down` / `mzkana_engine_key_up` — キーイベントを処理し `MzkanaResult` を返す
- `mzkana_engine_tick` — chord ウィンドウタイマーを進める
- `mzkana_engine_check_reload` — inotify でレイアウトファイルの変更を検出し自動リロード
- `mzkana_engine_reset` — フォーカス喪失・IM 切り替え時に状態をリセット

## 実装フェーズ

| Phase | 内容 | 状態 |
|---|---|---|
| 1 | 状態機械・設定パーサ・CLI（全シフト方式・漢直） | ✅ 完了 |
| 2 | Mozc IPC クライアント・`mozc-run` サブコマンド | ✅ 完了 |
| 3 | Fcitx5 アドオン化（C++ シム + preedit 同期 + ホットリロード） | ✅ 完了 |
| 4 | 設定 GUI（egui） | 未着手 |

## テスト

```sh
cargo test
```

65 件のテストがあります。Mozc のインストールは不要です。

## ライセンス

[LICENSE](LICENSE) を参照してください。
