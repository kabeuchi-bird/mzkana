# MzKana

Fcitx5 上で動作する、かな配列・漢直入力エンジンです。  
変換には Mozc を流用し、入力ロジックのみを純 Rust で実装します。

## 特徴

- **配列不問** — TOML 設定ファイルを差し替えるだけで任意のかな配列を使用できます
- **6 つのシフト方式に対応** — 通常シフト / 同時シフト / 前置シフト / 後置シフト / 相互シフト / センターシフト
- **3 キー以上の同時シフト** — 濁拗音・外来音（`ぎゃ` `ふぁ` 等）も chord で表現可能。最長一致で 2 キー部分集合（`きゃ`）から 3 キー（`ぎゃ`）へ自動アップグレード
- **漢直サポート** — T-code / TUT-code など 2 ストローク漢字直接入力に対応
- **投機的送信** — 入力をリアルタイムに Mozc へ送り、後から確定・書き換え（BS-rewrite）。同時打鍵は `chord_window_ms` の時間枠内のキーのみを対象に判定
- **候補ウィンドウ** — 予測変換候補（preedit 下）と変換候補（Mozc 風縦型）を表示。数字キーで直接選択、Space/矢印で候補送り
- **Mozc IPC クライアント** — Unix ドメインソケット経由で mozc_server に直接接続。起動していなければ自動起動し、切断時は次のキーイベントで自動再接続
- **UI 非凍結保証** — Mozc IPC は専用ワーカースレッドで実行し、1 打鍵あたり 150ms のハードタイムアウトを適用。mozc_server が遅延・ハングしてもデスクトップ入力は固まりません
- **機能キー出力** — `!Return` / `!Tab` など機能キーをグリッドや chord に埋め込み可能
- **修飾キー記法** — `C-z`（Ctrl+z）/ `S-!Left`（Shift+左）などを出力トークンとして指定可能
- **複数キー modifier** — 1 つの modifier を複数の起動キーに割り当て可能（`key = ["space", "henkan"]`）
- **エイリアス / 複数トークン出力** — `[[alias]]` で名前付きシーケンスを定義し、グリッドや chord から参照
- **Fcitx5 アドオン** — C++ シムレイヤー経由でインライン preedit・コミット・候補ウィンドウ・ホットリロードに対応
- **Mozc ステータス表示** — Fcitx5 ステータスバーに「MzKana（Mozc）」／「MzKana（変換エンジン未起動）」を表示

## リポジトリ構成

```
mzkana/
├── crates/
│   ├── mzkana-core/        # 状態機械・設定パーサ・Mozc IPCクライアント（ライブラリ）
│   │   ├── src/mozc/       # prost生成protobuf型 + UDSクライアント + ワーカースレッド
│   │   └── protocol/       # vendor した Mozc commands.proto（ビルド時に prost-build で生成）
│   ├── mzkana-cli/         # 検証・デバッグ用 CLI ツール
│   └── mzkana-ffi/         # C ABI ラッパー（cbindgen で mzkana.h を生成）
├── fcitx5-addon/           # Fcitx5 C++ アドオン
│   ├── src/engine.cpp/.h   # InputMethodEngineV2 実装
│   ├── data/               # mzkana.conf（アドオン登録）/ mzkana-im.conf（入力メソッド登録）
│   └── cmake/include/      # スタブエクスポートヘッダ（-dev 不要ビルド用）
└── layouts/                # サンプル配列定義
    ├── tsuki-2-263.toml    # 月配列 2-263 式（前置シフト）
    ├── shin-geta.toml      # 新下駄配列（同時シフト）
    ├── naginata-v17.toml   # 薙刀式 v17（センターシフト + 相互/3キー同時シフト）
    ├── jis_x_6004.toml     # 新 JIS 配列
    └── t-code.toml         # T-code（漢直）
```

> JSON Schema は `mzkana-cli schema` で標準出力に生成できます（`schemas/` ディレクトリは持ちません）。

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
# 使いたい配列を必要なだけコピー（複数可）
cp layouts/*.toml ~/.config/fcitx5/conf/mzkana/
```

このディレクトリ配下の `*.toml` が configtool の配列ドロップダウン（後述）に列挙
されます。どれを使うかは configtool で選択します。既定値は `layout.toml` なので、
従来どおり 1 ファイルだけ `layout.toml` として置く運用も可能です。

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

`fcitx5-configtool` の **「アドオン」タブ** → `MzKana` の行の **設定（歯車）ボタン**
を開くと、**「配列ファイル」ドロップダウン** が表示されます。
`~/.config/fcitx5/conf/mzkana/` 配下の `*.toml` が列挙されるので、使いたい配列を
選んで **「適用」** を押すと、その場で（fcitx5 の再起動なしに）切り替わります。

> ドロップダウンに出る候補はディレクトリ走査で動的に決まります。新しい `.toml` を
> 置いたら configtool を開き直すと一覧に反映されます。

レイアウトファイルの配置方法と直接編集時の自動リロードについては、上記の
「アドオン設定ファイルの配置」と「ホットリロード」の項を参照してください。

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
cargo run -p mzkana-cli -- run layouts/tsuki-2-263.toml --keys "a s"

# 前置シフト（d w → ひ）や 3 キー同時シフト（薙刀式 ぎゃ）も確認できる
cargo run -p mzkana-cli -- run layouts/tsuki-2-263.toml --keys "d w"
cargo run -p mzkana-cli -- run layouts/naginata-v17.toml --keys "w+h+j"

# 同時押し（+）でコードを表現
cargo run -p mzkana-cli -- run layouts/shin-geta.toml --keys "f+j"

# キーアップを ^ で表現
cargo run -p mzkana-cli -- run layouts/naginata-v17.toml --keys "space q space^"

# Mozc サーバに接続してキーシーケンスを送り、preedit/result を確認
cargo run -p mzkana-cli -- mozc-run layouts/naginata-v17.toml --keys "k a s d"

# Mozc ソケットのパスを明示する場合
cargo run -p mzkana-cli -- mozc-run layouts/naginata-v17.toml \
    --socket ~/.mozc/session.sock --keys "k a s d"

# JSON Schema を出力（エディタの補完設定に利用）
cargo run -p mzkana-cli -- schema
```

### `run` の出力例

```text
# a s（単打 2 文字）
send_kana(は)
send_kana(か)
[preedit] はか
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
key             = "space"          # 複数キーも可: key = ["space", "henkan"]
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
| `C-z` | Ctrl+z を送信（Mozc 経由；消費されなければアプリへ転送） |
| `S-!Left` | Shift+左矢印を送信（変換中は文節区切り調整） |
| `S-C-s` | 修飾子は重ねて指定可能（`S-` / `C-` / `A-` / `M-`） |
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

### 修飾キー記法

修飾プレフィックスを `!` キー名またはプレーンキーの前に付けます。プレフィックスは重ねられます。

| プレフィックス | 修飾キー |
|---|---|
| `S-` | Shift |
| `C-` | Ctrl |
| `A-` | Alt |
| `M-` | Super（Meta） |

```toml
# 例
output = "C-z"        # Ctrl+z（アプリ Undo またはMozc Undo）
output = "S-!Left"    # Shift+左矢印（Mozc 変換中は文節縮小）
output = "S-C-s"      # Shift+Ctrl+s
output = "A-!F4"      # Alt+F4
```

修飾キーは Mozc 経由で送信されます。Mozc が消費した場合（変換中のカーソル移動など）はそのまま preedit を更新し、消費されなかった場合はアプリケーションへ転送（`ic->forwardKey()`）します。

## Mozc IPC について

`mzkana-core` は Mozc の `commands.proto` を使って `mozc_server` と直接通信します。

- **接続先**: Linux 抽象ソケット（`/proc/net/unix` から自動検出）、フォールバックとして `~/.mozc/session.sock`（`--socket` で変更可）
- **プロトコル**: raw `Input` protobuf バイト列を送信 → `shutdown(SHUT_WR)` で終端通知 → raw `Output` protobuf バイト列を EOF まで受信（長さプレフィックスなし、`unix_ipc.cc` / `session_server.cc` で確認済み）
- **認証**: SO_PEERCRED によるカーネル UID 照合（mozc_server が同 UID かを検証）
- **protobuf**: 公式 `commands.proto`（および推移的 import）を vendor し、`prost-build` + `protoc-bin-vendored` でビルド時にコード生成（システム protoc 不要・ネットワーク不要）
- **セッション管理**: 接続時に `CREATE_SESSION` → `TURN_ON_IME`（HIRAGANA 合成モードへ初期化。既定の DIRECT のままでは入力が composer に入らない）、切断時に `DELETE_SESSION` を自動実行
- **ワーカースレッド**: IPC は専用スレッドで実行し、UI スレッドは 150ms のハードタイムアウトで応答待ち。超過時は接続を破棄して次イベントで再接続し、当該キーは未確定のまま素通し（ソケット自体の read/write タイムアウトは 1 秒）
- **自動起動**: `mozc_server` が未起動の場合、`/usr/lib/mozc/mozc_server` などを検索して自動起動（最大 500 ms 待機）
- **自動再接続**: 切断検出後、次のキーイベントで 5 秒バックオフ付きで再接続を試みる

## Fcitx5 アドオンのアーキテクチャ

```text
キーイベント
    │
    ▼
fcitx5-mzkana.so  (C++, InputMethodEngineV2)
    │  key_name + shift フラグに正規化
    ▼
libmzkana.so  (Rust FFI, mzkana-ffi)
    │  MzkanaResult { consumed, preedit, commit,
    │                 passthrough_key,
    │                 forward_key, forward_mods }
    ▼
MzkanaEngine  (mzkana-core)
    ├─ 状態機械（シフト / chord / 漢直）
    └─ Mozc IPC クライアント（preedit 同期・投機的送信・自動再接続）
```

- `mzkana_engine_key_down` / `mzkana_engine_key_up` — キーイベントを処理し `MzkanaResult` を返す
- `mzkana_engine_tick` — 内部タイマーを進める（chord 確定・期限切れ pending の除去・複合出力末尾の emit）。C++ 層が preedit 非空の間だけ約 10ms 周期の fcitx5 タイマー（`EventSourceTime`）から呼び出す
- `mzkana_engine_candidate_count` / `_candidate` / `_focused_index` — 直近の Mozc 出力の候補（予測・変換）を取得（C++ が候補ウィンドウを構築）
- `mzkana_engine_select_candidate` — 候補 id を指定して確定（数字キー選択。負 id は no-op）
- `mzkana_engine_check_reload` — inotify でレイアウトファイルの変更を検出し自動リロード
- `mzkana_engine_reset` — フォーカス喪失・IM 切り替え時に状態をリセット
- `mzkana_engine_mozc_available` — Mozc 接続状態を返す（ステータスバー表示に使用）

`forward_key` / `forward_mods` は修飾キートークン（`C-z` 等）が Mozc に消費されなかった場合に設定され、C++ 層が `ic->forwardKey()` でアプリへ転送します。

## 実装フェーズ

| Phase | 内容 | 状態 |
|---|---|---|
| 1 | 状態機械・設定パーサ・CLI（全シフト方式・漢直） | ✅ 完了 |
| 2 | Mozc IPC クライアント・`mozc-run` サブコマンド | ✅ 完了 |
| 3 | Fcitx5 アドオン化（C++ シム + preedit 同期 + ホットリロード） | ✅ 完了 |
| 4 | 候補ウィンドウ（予測・変換候補） | Rust 実装済み（テスト済み）/ C++ 実機ビルド要 |
| 5 | 設定 GUI（fcitx5-configtool 連携で配列ファイル選択） | 未着手 |

## テスト

```sh
cargo test
```

105 件のテストがあります。Mozc のインストールは不要です。

## ライセンス

[LICENSE](LICENSE) を参照してください。
