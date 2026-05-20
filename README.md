# MzKana

Fcitx5 上で動作する、かな配列・漢直入力エンジンです。  
変換には Mozc を流用し、入力ロジックのみを純 Rust で実装します。

## 特徴

- **配列不問** — TOML 設定ファイルを差し替えるだけで任意のかな配列を使用できます
- **6 つのシフト方式に対応** — 通常シフト / 同時シフト / 前置シフト / 後置シフト / 相互シフト / センターシフト
- **漢直サポート** — T-code / TUT-code など 2 ストローク漢字直接入力に対応
- **投機的送信** — 入力をリアルタイムに Mozc へ送り、後から確定・書き換え
- **機能キー出力** — `!Return` / `!Tab` など機能キーをグリッドや chord に埋め込み可能
- **エイリアス / 複数トークン出力** — `[[alias]]` で名前付きシーケンスを定義し、グリッドや chord から参照

## リポジトリ構成

```
mzkana/
├── crates/
│   ├── mzkana-core/   # 状態機械・設定パーサ（ライブラリ）
│   └── mzkana-cli/    # 検証・デバッグ用 CLI ツール
├── layouts/           # サンプル配列定義
│   ├── tsuki-2-263.toml   # 月配列 2-263 式（前置シフト）
│   ├── shin-geta.toml     # 新下駄配列（同時シフト）
│   ├── naginata-v17.toml  # 薙刀式 v17（センターシフト + 相互シフト）
│   └── t-code.toml        # T-code（漢直）
└── schemas/           # JSON Schema（エディタ補完用）
```

## ビルド

```sh
cargo build
```

## CLI ツール

```sh
# 配列ファイルの検証
cargo run -p mzkana-cli -- validate layouts/tsuki-2-263.toml

# キーシーケンスを流して出力アクションを確認
cargo run -p mzkana-cli -- run layouts/tsuki-2-263.toml --keys "d w"

# コードを同時押し（+）で表現
cargo run -p mzkana-cli -- run layouts/shin-geta.toml --keys "f+j"

# キーアップを ^ で表現
cargo run -p mzkana-cli -- run layouts/naginata-v17.toml --keys "space q space^"

# JSON Schema を出力（エディタの補完設定に利用）
cargo run -p mzkana-cli -- schema
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
1 。   か   た   こ   さ   ら   ち   く   つ   ，
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
3 ぅ   ひ   そ   み   .    ゆ   に   ま   の
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

## テスト

```sh
cargo test
```

## ライセンス

[LICENSE](LICENSE) を参照してください。
