# MzKana 設計書 v0.2

Fcitx5 上で動作するかな配列・漢直入力エンジンの設計仕様。

---

## 目次

1. [概要](#1-概要)
2. [アーキテクチャ](#2-アーキテクチャ)
3. [シフト方式の統一抽象](#3-シフト方式の統一抽象)
4. [キー識別子](#4-キー識別子)
5. [状態機械アルゴリズム](#5-状態機械アルゴリズム)
6. [設定ファイル仕様](#6-設定ファイル仕様)
7. [漢直サポート](#7-漢直サポート)
8. [Mozc 連携](#8-mozc-連携)
9. [競合検出](#9-競合検出)
10. [ホットリロード](#10-ホットリロード)
11. [Preedit 表示戦略](#11-preedit-表示戦略)
12. [設定パラメータ一覧](#12-設定パラメータ一覧)
13. [実装構成](#13-実装構成)
14. [実装フェーズ](#14-実装フェーズ)

---

## 1. 概要

### 目的

任意のかな配列および漢直配列を、設定ファイルの切り替えだけで利用できる Fcitx5 入力メソッドを構築する。変換エンジンには Mozc を流用し、入力側ロジックのみを独自実装する。

### サポートするシフト方式

| 方式 | 説明 |
|---|---|
| 通常シフト | Shift キーを併用する |
| 同時シフト | 既定ミリ秒以内に複数キーを同時押し、順序固定 |
| 前置シフト | トリガキーを押した直後の 1 キーにシフトを適用 |
| 後置シフト | 対象キー → トリガキーの順で 1 キーにシフトを適用 |
| 相互シフト | 同時シフトの対称版、押下順序に依存しない |
| センターシフト | 任意キーの押下中、別レイヤーが継続的に有効（Modifier 扱い） |

### 設計目標

- 設定ファイルで配列を完全に表現できること
- 設定の可読性・可書性が高いこと
- かな配列と漢直配列を統一的に扱えること
- Mozc の変換品質をそのまま享受できること

---

## 2. アーキテクチャ

```
                ┌─────────────────────────────────────┐
                │  Application (Wayland/X11 client)   │
                └─────────────────────────────────────┘
                                ↑↓ text / preedit
                ┌─────────────────────────────────────┐
                │  Fcitx5 core                        │
                └─────────────────────────────────────┘
                                ↑↓ KeyEvent / commit
   ┌────────────────────────────────────────────────────────┐
   │  MzKana addon                                         │
   │  ┌──────────────────────────────────────────────────┐  │
   │  │  C++ thin shim (FcitxInputMethodEngineV2 impl)   │  │
   │  │  ├ KeyEvent → keysym 正規化（XKB level 0）       │  │
   │  │  └ core への FFI 呼び出し                         │  │
   │  └──────────────────────────────────────────────────┘  │
   │                       ↕ cbindgen FFI                   │
   │  ┌──────────────────────────────────────────────────┐  │
   │  │  Rust core (mzkana-core)                         │  │
   │  │   ├─ Config loader (TOML)                         │  │
   │  │   ├─ Shift state machine                          │  │
   │  │   ├─ Output dispatcher                            │  │
   │  │   │    ├→ kana → Mozc preedit                     │  │
   │  │   │    └→ 漢直結果 → 直接 commit                  │  │
   │  │   └─ Mozc client (protobuf over UDS)              │  │
   │  └──────────────────────────────────────────────────┘  │
   └────────────────────────────────────────────────────────┘
                                ↑↓ protobuf IPC
                ┌─────────────────────────────────────┐
                │  mozc_server (既存バイナリ流用)     │
                └─────────────────────────────────────┘
```

### 設計判断

- **fcitx5-mozc をフォークしない**。独立アドオンとして mozc_server に直接 IPC。fcitx5-mozc と共存可能。
- **ローマ字テーブルは使わない**。Mozc 側にはかな入力モードで送る（同時/相互シフトはローマ字テーブルでは表現不能）。
- **C++ シムは最小限**。Rust core で状態機械・設定・IPC を全て担う。
- **言語**: Rust（core） + C++（fcitx5 plumbing のみ）。

---

## 3. シフト方式の統一抽象

各シフト方式を別々に実装すると組合せ爆発する。**全方式を「制約付きキー列マッチング」に還元**する。

```rust
struct Rule {
    pattern: Pattern,
    constraints: Constraints,
    output: Output,
    priority: u32,
}

enum Pattern {
    Single(KeyId),                  // 単打
    Sequence(Vec<KeyId>),           // 前置/後置シフト
    Chord(BTreeSet<KeyId>),         // 同時/相互シフト
    Modified(ModifierId, Box<Pattern>),  // 通常シフト/センターシフト
}

struct Constraints {
    max_overlap_ms: Option<u32>,    // chord 用：押下間の許容遅延
    layer: Option<LayerId>,
    symmetric: bool,                // 相互シフト用
}

enum Output {
    Kana(String),                   // Mozc preedit へ送る
    Direct(String),                 // 漢直、直接 commit
    Passthrough,                    // そのまま fcitx5 へ返す
}
```

### 各方式のマッピング

| 方式 | Pattern | symmetric | max_overlap_ms |
|---|---|---|---|
| 通常シフト | `Modified(Shift, Single(K))` | - | - |
| 同時シフト | `Chord({S, K})` | false | 50 |
| 前置シフト | `Sequence([P, K])` | - | - |
| 後置シフト | `Sequence([K, P])` | - | - |
| 相互シフト | `Chord({A, B})` | **true** | 80 |
| センターシフト | `Modified(Hold(K), Single(K2))` | - | - |

相互シフトの本質は「どちらが先でも同じ出力」なので、`Chord` に `symmetric: true` フラグを付ければ単一ロジックで扱える。

---

## 4. キー識別子

### XKB の keysym を直接利用する

fcitx5 が addon に渡す KeyEvent は既に XKB 変換後の keysym を持っている。これをそのまま識別子化する。専用の scancode → 文字テーブルは不要。

**シフトによる文字変化を吸収するため、level 0（修飾なし）の keysym を取得**して使う。

```cpp
// fcitx5-addon 側
auto code = event.key().code();
auto base = xkb_state_key_get_one_sym_for_level(
    xkb_state, code, /*layout*/ 0, /*level*/ 0);
auto mods = event.key().states();
auto is_repeat = event.key().isRepeat();

auto ident = keysym_name(base);   // "a", "q", "comma", "yen" ...
core_feed_key(ident, mods, is_repeat);
```

`xkbcommon` は fcitx5 が既に持っているので追加依存なし。

### 識別子化規則

```
level 0 sym               → identifier
──────────────────────────────────────
XK_a..XK_z                → "a".."z"
XK_0..XK_9                → "0".."9"
XK_comma, XK_period       → "comma", "period"
XK_space, XK_BackSpace    → "space", "bs"
XK_yen, XK_underscore     → "yen", "underscore"
XK_Muhenkan, XK_Henkan    → "muhenkan", "henkan"
XK_Hiragana_Katakana      → "kana"
それ以外                  → XKB symbol name を小文字化
```

JIS 固有キーも XKB が認識するため、専用テーブル不要。

### 配列別の含意（物理 Q キー押下時の識別子）

| ユーザ環境 | identifier | 標準同梱 TOML が動くか |
|---|---|---|
| US QWERTY | `"q"` | ✓ |
| JIS QWERTY | `"q"` | ✓ |
| Dvorak | `"apostrophe"` | ✗ Dvorak 用 TOML が必要 |
| Colemak | `"f"` | ✗ Colemak 用 TOML が必要 |

Dvorak/Colemak ユーザは自分の XKB 配列に合わせた TOML を用意する。これは IME として正しい挙動。

### 修飾キーの扱い

```
Ctrl/Alt/Super/Meta が押下中 → 即 passthrough（評価スキップ）
Shift のみ押下中            → 通常シフト層のルール評価対象に加える
CapsLock 状態               → settings.caps_lock_behavior に従う
```

### キーリピート

XKB はリピート時にも同じ sym を送る。同時/相互シフト判定で誤爆するため、リピート中は pending_keys に積まない。

---

## 5. 状態機械アルゴリズム

### 内部状態

```rust
struct StateMachine {
    pending_keys: Vec<(KeyId, Instant)>,
    tentative_buffer: Vec<TentativeChar>,    // Mozc に送ったが書換可能な文字
    active_modifiers: BTreeSet<ModifierId>,
    direct_trigger_active: bool,             // 7 章の漢直トリガー状態
    mozc_mode: MozcMode,                     // COMPOSITION | CONVERSION
    chord_timer: Option<Instant>,
    rules: RuleIndex,                        // prefix trie + chord index
}

struct TentativeChar {
    kana: String,             // Mozc preedit に送った文字（1 つの kana、ゔ等の合成済み含む）
    source_keys: Vec<KeyId>,  // どのキー（列）から生成したか
    sent_at: Instant,
    rewrite_deadline: Option<Instant>,  // chord 候補がある場合のみ Some
}
```

### 核心：Speculative execution + BS-rewrite

未確定中のキーは常に「最善推定」の kana を Mozc preedit に即時送信し、より優先される規則が後からマッチしたら BS で書き換える。これにより、シフト方式に関わらずユーザは即時の視覚フィードバックを得つつ、任意の時間後の解決を許容できる。

### イベント処理

```
KeyDown(k, t):
  if is_repeat:
    return

  if k は modifier 定義に該当:
    active_modifiers.insert(modifier_id)
    return

  if k は direct_trigger.keys に該当:
    direct_trigger_active を更新（kind に応じて hold/toggle）
    pending_keys と tentative_buffer をクリア
    return

  if mozc_mode == CONVERSION:
    tentative_buffer.clear()  # 書換機会消失
    （Mozc は新たな入力で COMPOSITION に戻る）

  pending_keys.push((k, t))

  # 候補規則の評価
  candidates = rules.match(pending_keys, active_modifiers, direct_trigger_active)

  match candidates:
    [single_complete] if 唯一完全マッチ and 延長可能性なし:
      → send_resolved(rule)   # 即確定、書換不要

    sequence/chord 候補あり:
      → speculative_emit(k)   # 即時 speculative 送信

KeyUp(k, t):
  if k は modifier/direct_trigger:
    active_modifiers.remove or direct_trigger_active 更新
    return
  # chord 解決チェック（後述）

ChordTimer 発火:
  tentative_buffer 内、deadline 超過の chord 機会を確定
  （該当の TentativeChar の rewrite_deadline を None に変更）
```

### speculative_emit

```
speculative_emit(k):
  if direct_trigger_active:
    # 漢直モード中：silent、tentative なし
    direct_rules.match_or_extend(pending_keys)
    if 完成: emit_direct_kanji(); pending_keys.clear()
    else if 候補あり: 待機
    else: pending_keys.clear()  # 無効列、破棄
    return

  # かなモード
  base = base_layer.lookup(k, active_modifiers)
  if base.is_none():
    # base 層に割当なし（pure trigger key 等）→ tentative 送信しない
    return

  send_to_mozc(KeyString(base.kana))
  tentative_buffer.push(TentativeChar {
    kana: base.kana,
    source_keys: vec![k],
    sent_at: t,
    rewrite_deadline: 
      if chord 候補あり: Some(t + chord_window_ms)
      else if sequence 候補あり: None  # sequence は永久書換可能
      else: 即時 None（書換機会なし、確定済み）
  })
  
  # sequence 規則の継続候補があれば pending_keys に保持
  # chord 規則の継続候補があれば pending_keys に保持
  # 何も継続候補がなければ pending_keys.clear()
```

### 規則マッチ時の BS-rewrite

```
on_rule_match(rule):
  # tentative_buffer のうち rule.source_keys に該当する文字を取消
  affected = tentative_buffer の末尾から source_keys 集合を含む TentativeChar 群
  
  for _ in affected:
    send_to_mozc(KeyCode(BackSpace))
  tentative_buffer.truncate(affected.len() 分前)
  
  # rule の出力を新たに送信
  send_to_mozc(KeyString(rule.output))
  tentative_buffer.push(TentativeChar {
    kana: rule.output,
    source_keys: rule.source_keys,
    rewrite_deadline: 
      これ以上書換可能な規則があれば Some、なければ None
  })
  
  pending_keys.clear()
```

### chord_window_ms の意味

旧仕様の「chord 判定のために待機する時間」ではなく、**「speculative 送信後、chord による BS-rewrite を受け付ける時間」** として再定義。

```
'f' 押下 t=0:
  speculative_emit → "は" を Mozc に送信、preedit: "は"
  tentative_buffer 末尾.rewrite_deadline = t + chord_window_ms

'j' 押下 t=20ms（window 内）:
  chord [f, j] → "を" マッチ
  on_rule_match: BS、"を" 送信、preedit: "を"

'j' 押下 t=200ms（window 外）:
  "は" は既に確定（rewrite_deadline 過ぎ）
  'j' は新たな単打として処理、preedit: "はと"
```

### sequence のタイムアウトなし

sequence 候補がある間、`rewrite_deadline = None` で永久書換可能。

```
'd' 押下: speculative "か" 送信、preedit: "か"
  sequence [d, k]→"れ" 候補あり、rewrite_deadline = None

  ┌ 'k' 押下（任意時間後）→ "か" を BS、"れ" を送信
  ├ 'a' 押下 → [d, a] は規則なし
  │           "か" は確定、'a' が speculative "あ" 送信、preedit: "かあ"
  └ Space 押下（変換開始）→ mozc_mode = CONVERSION
                            tentative_buffer クリア、'd' は "か" として確定
```

### BS キー入力の扱い

ユーザが BackSpace を押下：

```
KeyDown(BackSpace):
  if tentative_buffer 非空:
    tentative_buffer.pop()
    pending_keys.clear()  # 対応する pending もクリア
  send_to_mozc(KeyCode(BackSpace))  # Mozc 側 preedit からも 1 文字消える
```

### Mozc CONVERSION 遷移

Mozc から返る `Output.mode` を監視：

```
mozc_response.mode == CONVERSION:
  tentative_buffer.clear()
  pending_keys.clear()
  # 以降の入力は完全に新規 sequence として処理
```

### 解決ルール

```
1. priority 明示指定があれば最優先
2. longest match > shorter match
3. chord > sequence > single
4. cross-context は競合扱いしない
   （layer / pattern_kind / activation_scope が違えば共存可、9 章参照）
5. 同 context で kana と direct が衝突する場合は kana 優先
   （`[direct_trigger]` 未定義時のみ発生し得る、設定読込時に警告済み）
```

### roll-over 処理

speculative model では自動的に処理される：典型的なタイプミス（chord 未完成）も、各キーが即時 base 単打として preedit に積まれ、chord_window_ms 経過で確定する。チラつきもなく自然。

### Hold 判定（センターシフト用）

| 方式 | 動作 |
|---|---|
| `interrupt` | hold 対象キー押下中に他キー入力 → 即 hold 確定。離した時に他キー押下がなければ tap。 |
| `timeout` | 単純に N ms 超えれば hold。 |

`interrupt` は反応が良いが、単独 tap は他キー無しで離す必要あり。`timeout` は誤爆少ないが反応が鈍い。設定で選択可能。

Modifier 系の出力は最初から最終層の kana を送るため、書換は発生しない。

### 漢直 sequence の例外

`direct_trigger_active` の間は speculative 送信を行わない。漢直 sequence は完成するまで silent（preedit に何も出ない）。完成時に直接 commit を行う（7 章参照）。これは T-code 系ユーザの慣習に従う。

---

## 6. 設定ファイル仕様

### フォーマット

TOML をベースに、視覚的グリッド文字列でレイアウトを表現するハイブリッド形式。

### 完全例

```toml
# ============================================================
# MzKana layout file v1
# ============================================================

[meta]
name      = "月配列2-263式"
version   = "1.0"
author    = "..."
mode      = "kana"               # "kana" | "kanchoku" | "hybrid"
schema    = 1

[settings]
chord_window_ms       = 50
mutual_window_ms      = 80
caps_lock_behavior    = "shift"  # "shift" | "ignore" | "passthrough"
on_focus_change       = "preserve"  # "preserve" | "reset"
roll_over             = true

# ── Modifier 定義 ─────────────────────────────────
[[modifier]]
id              = "center"
key             = "space"
kind            = "hold"         # "hold" | "oneshot"
hold_detection  = "interrupt"    # "interrupt" | "timeout"
hold_timeout_ms = 150            # timeout 方式のみ参照
tap_action      = "send_key"

# ── 漢直トリガー（optional） ──────────────────────
[direct_trigger]
keys           = ["henkan"]
kind           = "hold"
hold_detection = "interrupt"
tap_action     = "passthrough"

# ── Layer 定義 ────────────────────────────────────
[[layer]]
id   = "base"
kind = "single"
# 行頭の数字は行番号（無視される、可読性のため）
# 列見出しは QWERTY 物理位置
grid = """
. q w e r t y u i o p
1 。 か た こ さ ら ち く つ ，
2 う し て け せ は と き い ん
3 ．  ひ そ こ み      ゆ に ま の
"""

[[layer]]
id      = "shift_d"
kind    = "prefix"
trigger = "d"
grid    = """
. q w e r t y u i o p
1 ぁ え り ゃ れ ぱ ぢ ぐ づ ぴ
2 を う゛ぃ ぬ ょ ふ ご げ ぞ ぼ
3 ぅ ひ゛そ こ み      ゆ に ま の
"""

[[layer]]
id       = "center_shift"
kind     = "modified"
modifier = "center"
grid     = """..."""

# ── 個別ルール（グリッド外） ──────────────────────
[[chord]]
keys      = ["f", "j"]
output    = "を"
symmetric = false

[[chord]]
keys      = ["a", "s"]
output    = "ざ"
symmetric = true                 # 相互シフト
window_ms = 80                   # 個別上書き

# ── 漢直エントリ ──────────────────────────────────
[[direct]]
sequence = ["k", "j"]
output   = "日"

[[direct]]
sequence = ["f", "j"]
output   = "本"
```

### JSON Schema

`schemas/layout.schema.json` をビルド時に自動生成。エディタ補完を効かせる。

### 配列モード

| `meta.mode` | 動作 |
|---|---|
| `kana` | 全出力を Mozc 経由でかな漢字変換 |
| `kanchoku` | 全エントリを `[[direct]]` 扱い、Mozc バイパス |
| `hybrid` | `[[direct]]` のみ直接 commit、他はかなとして Mozc 経由 |

---

## 7. 漢直サポート

### Speculative 対象外（silent 動作）

漢直 sequence は 5 章の speculative execution 機構の**例外**として、完成まで preedit に何も送らない。T-code 系ユーザは「2 打打って漢字が出る」のサイレントな挙動を期待しており、途中経過の kana が見えると混乱するため。

```
direct sequence 中の挙動:
  ├ 1 ストローク目押下 → preedit 変化なし（tentative 送信せず）
  ├ 2 ストローク目押下 → sequence 完成、漢字直接 commit
  └ sequence 不一致     → pending 破棄、特に何も commit せず
```

### 完成時の commit 手順

```
[[direct]] エントリが完成 → 以下を順次実行:
  1. tentative_buffer が非空なら全て BS で消す（kana モードからの遷移時のみ発生）
  2. Mozc に SUBMIT コマンド送信（confirmed preedit を変換確定）
  3. fcitx5 経由で漢字を直接 commit_text
  4. pending_keys、tentative_buffer クリア
```

「変換中に漢直を打つと未変換のかなが先に commit される」挙動になる点はドキュメントで明示。

### 既存配列の対応

- T-code: 全 `[[direct]]` で `mode = "kanchoku"`
- TUT-code: `hybrid` モードでかな + 漢直混在
- G-code: `hybrid`、補助漢字テーブルも別 layer として記述可能

### 漢直トリガー

`[[direct]]` を常時有効にせず、特定キーの押下中（または toggle）のみ有効化する仕組み。`hybrid` モードで「普段はかな、変換キー押している間だけ漢直」運用を可能にする。

```toml
[direct_trigger]
keys           = ["henkan"]       # 複数 OR 指定可（変換 と 無変換 両対応など）
kind           = "hold"           # "hold" | "toggle"
hold_detection = "interrupt"      # kind = "hold" 時のみ
hold_timeout_ms = 150
tap_action     = "passthrough"    # "passthrough" | "none"
```

### 動作

| トリガ状態 | 評価対象ルール | speculative | 非マッチキーの扱い |
|---|---|---|---|
| inactive | kana ルールのみ（`[[direct]]` は無視） | あり（5 章機構） | 通常の speculative + rewrite |
| active | `[[direct]]` のみ（kana ルールは抑止） | **なし**（silent） | 部分一致なら pending 保持、不一致なら破棄 |

### 状態遷移時の挙動

```
inactive → active:
  ├ pending_keys クリア
  ├ tentative_buffer クリア（残っていた kana は preedit に確定的に残る）
  └ 直接ルール評価開始（silent）

active → inactive:
  ├ pending_keys クリア（未完成 sequence は破棄）
  ├ tentative_buffer は空（active 中は積まないため）
  └ Mozc セッションは維持
```

トリガキー単独タップ（押下中に他キー入力なしで離した）時：

| `tap_action` | 動作 |
|---|---|
| `"passthrough"` | 元キー（変換、無変換等）を fcitx5 に送る |
| `"none"` | 何もしない |

### トリガ未定義時

`[direct_trigger]` セクションが無ければ `[[direct]]` は `meta.mode` の規定どおり常時有効（後方互換）。この場合 silent / speculative の境界は規則の種別で決まる：

- kana 規則のマッチ候補 → speculative 動作
- direct 規則のマッチ候補 → silent 動作
- 両方の候補が同時にある場合 → silent（direct が完成する可能性があるため待機）

### モード組合せ

| `meta.mode` | トリガあり | トリガなし |
|---|---|---|
| `kana` | 押下中のみ直接ルール（漢直エントリが空でも合法） | 直接ルール完全無効 |
| `hybrid` | 押下中のみ直接、それ以外はかな（speculative） | 両方常時有効、競合は kana 優先 |
| `kanchoku` | 押下中のみ直接、それ以外は passthrough | 直接ルール常時有効、全て silent |

---

## 8. Mozc 連携

### 接続方式

mozc_server に **protobuf over Unix Domain Socket** で直接 IPC。fcitx5-mozc を経由しない。

### ビルド時の処理

`prost-build` で `mozc/protocol/commands.proto` を取り込み、Rust 型を生成。

### 接続先

```
~/.mozc/session.sock
```

### キー送信の方式

ローマ字としてではなく、**確定したかな文字列を直接送る**：

```rust
let key_event = KeyEvent {
    key_code: None,
    key_string: Some("か".to_string()),
    input_style: InputStyle::DIRECT_INPUT,
    ..Default::default()
};
let output = mozc_client.send_key(key_event).await?;
```

Mozc 側からはローマ字入力かかな直接入力かを区別する必要がなく、変換品質はそのまま得られる。

### BS-rewrite プロトコル

5 章の speculative execution が要求する書換動作は、Mozc IPC では「BackSpace + 新 kana 送信」のシーケンスで実現する。

```rust
fn rewrite_tentative(&mut self, removed: usize, new_kana: &str) {
    // 1. 取消したい tentative 文字数分だけ BS を送る
    for _ in 0..removed {
        mozc_client.send_key(KeyEvent {
            key_code: Some(KeyCode::Backspace),
            input_style: InputStyle::DIRECT_INPUT,
            ..Default::default()
        }).await?;
    }
    // 2. 新しい kana を送る
    mozc_client.send_key(KeyEvent {
        key_string: Some(new_kana.to_string()),
        input_style: InputStyle::DIRECT_INPUT,
        ..Default::default()
    }).await?;
}
```

DIRECT_INPUT モードでは Mozc 自身は pending を持たないため、送信した key_string は即座に Mozc preedit の末尾に追加され、BackSpace は末尾の 1 文字を削除する。我々の `tentative_buffer` と Mozc preedit が一対一で対応する単純な不変条件が保てる。

### preedit / commit 同期

```
Mozc.Output.preedit    → fcitx5 の preedit 表示に反映（11 章の戦略に従う）
Mozc.Output.result     → fcitx5 経由で commit
Mozc.Output.candidates → fcitx5 の候補ウィンドウに反映
Mozc.Output.mode       → 状態機械の mozc_mode を更新
```

### CONVERSION モード遷移の検出

ユーザが Space 等を押して変換候補を開いた時、Mozc は `Output.mode = CONVERSION` を返す。状態機械側は以下を行う：

```rust
on_mozc_output(output: Output) {
    if output.mode == CONVERSION && self.mozc_mode != CONVERSION {
        self.tentative_buffer.clear();  // 書換機会消失
        self.pending_keys.clear();
        // 以降の文字キー入力は新規 sequence として処理
    }
    self.mozc_mode = output.mode;
}
```

CONVERSION 中は新たな kana キー入力で Mozc が自動的に COMPOSITION に戻る（候補確定 + 新規入力）ため、特殊な復帰処理は不要。

### 漢直 commit との相互作用

```rust
fn handle_direct_output(kanji: &str) {
    // 1. tentative_buffer が残っていれば全て BS で消す
    for _ in 0..self.tentative_buffer.len() {
        mozc_client.send_key(BackSpace).await?;
    }
    self.tentative_buffer.clear();

    // 2. Mozc preedit を確定（残っていれば変換した上で commit される）
    mozc_client.submit().await?;

    // 3. 漢直結果を直接 commit
    fcitx5.commit_text(kanji);

    // 4. 状態リセット
    self.pending_keys.clear();
}
```

---

## 9. 競合検出

### 競合の定義

```
context = (layer_id, pattern_kind, key_set, activation_scope)

activation_scope の値:
  "always"           : 常時アクティブなルール
  "trigger_active"   : [direct_trigger] 押下中のみアクティブ
  "modifier_active(id)" : 特定 modifier 押下中のみアクティブ

同じ context 内で異なる output を持つルールが 2 つ以上 → エラー
cross-context（activation_scope や layer が違う）は競合ではない
```

各ルールの activation_scope 決定：

| ルール種別 | `[direct_trigger]` 定義 | activation_scope |
|---|---|---|
| kana 系 (layer/chord) | あり | `"always"`（トリガ押下中は抑止されるが context 上は always） |
| kana 系 (layer/chord) | なし | `"always"` |
| `[[direct]]` | あり | `"trigger_active"` |
| `[[direct]]` | なし | `"always"` |
| modified layer | - | `"modifier_active(id)"` |

### トリガーキーまで含めた食い合い判定

`[direct_trigger]` 定義時、`[[direct]]` の有効キー列は **「トリガ押下 + sequence」** として扱う。つまり同じ sequence でも、トリガ要否が異なれば異 context として共存可能。

### エラーと警告の分類

| 種別 | 動作 |
|---|---|
| 同 context・完全一致・出力相違 | **エラー**。設定読込失敗。 |
| 異 context・完全一致 | 通知なし（正当な共存）。 |
| Prefix 競合（短 rule が長 rule の prefix）・同 context | 情報レベルで通知、解決規則で確定。 |
| `[direct_trigger]` 定義あり、同キー列で kana と direct 両存在 | 競合なし（時間的排他、トリガ要否が異なる） |
| `[direct_trigger]` 定義なし、同キー列で kana と direct 両存在 | **警告**。実行時は kana 優先で direct 不到達。 |
| トリガキーが他ロール（modifier, prefix trigger, base layer 等）にも出現 | **警告**。同一物理キーに複数役割、意図しない衝突の可能性。 |
| modifier キーが他ロールに出現 | **警告**。同上。 |

### 判定例

```
例 1: [direct_trigger] あり、トリガ要否でルール分離
─────────────────────────────────────────────
  [direct_trigger]  keys = ["henkan"]
  [[chord]]   keys = ["k","j"]  output = "も"
  [[direct]]  sequence = ["k","j"]  output = "日"

  → chord は activation_scope = "always"
    direct は activation_scope = "trigger_active"
  → 異 context、競合なし
  → 通常時は「も」、henkan 押下中は「日」


例 2: [direct_trigger] なし、同キー列で kana と direct 両定義
─────────────────────────────────────────────
  （direct_trigger 未定義）
  [[chord]]   keys = ["k","j"]  output = "も"
  [[direct]]  sequence = ["k","j"]  output = "日"

  → 両方 activation_scope = "always"
  → 同 context、警告
  → 実行時は kana 優先で「も」、漢直「日」は到達不能


例 3: トリガキーが modifier キーと重複
─────────────────────────────────────────────
  [direct_trigger]  keys = ["henkan"]
  [[modifier]]      id = "shift_henkan"  key = "henkan"

  → 警告。henkan に 2 つの役割が割当られている


例 4: トリガキーが base layer に出現
─────────────────────────────────────────────
  [direct_trigger]  keys = ["henkan"]
  [[layer]]  kind = "single"  grid = """... henkan に「ん」割当 ..."""

  → 警告。henkan 押下 → 即 direct モード遷移するため
    base 層の「ん」は到達不能


例 5: トリガキー定義違い、同 sequence で direct 複数
─────────────────────────────────────────────
  [direct_trigger]  keys = ["henkan", "muhenkan"]
  [[direct]]  sequence = ["k","j"]  output = "日"
  [[direct]]  sequence = ["k","j"]  output = "本"

  → エラー。同 context（trigger_active）で出力相違
```

### 検出のタイミング

設定読込時に静的解析。`mzkana-cli validate <layout.toml>` で事前チェック可能。

### 通知方法

```
読込成功 + 警告あり → fcitx5 notification で警告内容表示
読込失敗            → 旧設定維持 + エラー詳細を notification で表示
```

---

## 10. ホットリロード

### 監視対象

```
~/.config/fcitx5/conf/mzkana/
```

配下の `.toml` ファイルを `notify` crate で監視。

### 動作

```
TOML 変更検知
  ├ パース + 検証成功 → ライブ差し替え、入力中の状態はクリア
  │                     fcitx5 notification で告知
  └ 失敗               → 旧設定維持
                          エラー詳細を notification で表示
```

### 状態保持

リロード時、`pending_keys`、`tentative_buffer`、`active_modifiers` をクリア。Mozc セッションは維持。tentative が残っていれば BS で除去してから設定を差し替える。

---

## 11. Preedit 表示戦略

### 背景

クライアントアプリが preedit のインライン表示に対応しないケースがある。発生環境：

| 環境 | 理由 |
|---|---|
| 旧 `xterm`（XIM root mode） | on-the-spot 非対応 |
| パスワード等 sensitive フィールド | `CapabilityFlag::PasswordOrSensitive` で preedit 無効化 |
| 一部 Wayland クライアント | `zwp_text_input_v3` 未実装 or 不完全 |
| SDL / Vulkan ネイティブアプリ | IME プロトコル未対応 |
| Electron / JetBrains の一部 | 実装が壊れている |
| `dmenu` / 一部 rofi 入力欄 | IME 非対応 |

これらでもユーザが入力中の文字を視認できるよう、fcitx5 の Input Panel（変換窓）にフォールバック表示する。

### Input Panel の構造

fcitx5 の Input Panel は単一フローティングウィンドウに複数領域を持つ：

```
┌─────────────────────────────────┐
│  Aux Up                          │
├─────────────────────────────────┤
│  Preedit         ← preedit 表示  │
├─────────────────────────────────┤
│  1. こんにちは   ← 変換候補       │
│  2. 今日は                        │
│  3. ...                          │
├─────────────────────────────────┤
│  Aux Down                        │
└─────────────────────────────────┘
```

`InputPanel::setPreedit()` で渡したテキストはこの「Preedit 領域」に表示され、Mozc から候補が返ってくればその下に並ぶ。

### 4 段階の戦略

```rust
enum PreeditStrategy {
    ClientInline,         // CapabilityFlag::Preedit あり、inline 表示
    PanelPreedit,         // inline 不可、変換窓内に表示
    BufferOnly,           // パネル描画も不可、内部バッファのみ、確定時に commit
    PassthroughImmediate, // sensitive、addon バイパス
}

fn select_strategy(caps: CapabilityFlags) -> PreeditStrategy {
    if caps.contains(PasswordOrSensitive)  { return PassthroughImmediate; }
    if caps.contains(Preedit)              { return ClientInline; }
    if fcitx5_panel_available()            { return PanelPreedit; }
    BufferOnly
}
```

### 各戦略の動作

| 戦略 | 表示位置 | 動作 |
|---|---|---|
| `ClientInline` | アプリ内インライン | `setClientPreedit()` 経由でクライアントに送信 |
| `PanelPreedit` | fcitx5 フローティング窓 | `setPreedit()` 経由でパネル内 Preedit 領域に表示 |
| `BufferOnly` | 表示なし | 内部 `tentative_buffer` のみ保持、確定時に一括 commit |
| `PassthroughImmediate` | アプリにそのまま | 状態機械をバイパス、生キーを commit |

### Speculative execution との関係

5 章の speculative + BS-rewrite 機構は、`ClientInline` と `PanelPreedit` では Mozc preedit に対する書換として可視に動作する。`BufferOnly` では Mozc preedit が表示されないが、**内部の `tentative_buffer` と Mozc preedit の状態は同じロジックで維持**される（書換結果が変わらないため最終出力は同等）。

| 戦略 | speculative の可視性 | rewrite の動作 |
|---|---|---|
| `ClientInline` | アプリ inline に反映、書換が見える | 通常通り BS + 再送 |
| `PanelPreedit` | 変換窓内に反映、書換が見える | 通常通り BS + 再送 |
| `BufferOnly` | 不可視 | 内部状態としては動作、ユーザには変換窓もアプリも変化なし |
| `PassthroughImmediate` | 該当せず | 状態機械をバイパスするため tentative なし |

`BufferOnly` モードでは「入力中は何も見えない、確定操作で一括 commit」となるが、最終的に commit される kana 列は preedit 表示時と完全に同等。

`PanelPreedit` モードのシーケンス：

```
KeyDown(a) → 状態機械 → speculative "あ"
  ↓
mozc_client.send_key("あ") → preedit="あ", candidates=[...]
  ↓
ic->inputPanel().setPreedit(Text("あ"));
ic->inputPanel().setCandidateList(...);
ic->updateUserInterface(UserInterfaceComponent::InputPanel);
  ↓
[変換窓内に "あ" + 候補リスト表示]
```

### BufferOnly が発動する条件

パネル描画自体が不可能な極稀ケース：

| 環境 | 理由 |
|---|---|
| `wlr-layer-shell` 非対応 Wayland コンポジタ | パネル描画手段なし |
| VRChat 等の特殊フルスクリーン | layer-shell が overlay 上に出ない |
| `fcitx5 --disable-ui` | ユーザによる明示無効化 |

niri は layer-shell 対応のため発生しない。

### sensitive フィールドの扱い

パスワード入力欄では IME 自体を効かせないことが正しい挙動。状態機械をバイパスし、生キーをそのまま commit する。設定で「sensitive でも内部バッファのみ持つ」モードに切り替え可能。

### 漢直モードへの影響

`mode = "kanchoku"` は本質的に preedit を使わず直接 commit のため、本章の戦略選択の影響を受けない。`mode = "hybrid"` ではかな部分のみが本戦略の対象となる。

### 設定

```toml
[settings]
preedit_fallback = "panel"
# "client"  : client preedit のみ、cap 無ければ無表示
# "panel"   : client → panel フォールバック（推奨デフォルト）
# "buffer"  : 全環境で内部バッファのみ、確定時 commit
# "auto"    : capability に応じて全段階を自動選択（panel と同等）

sensitive_field_behavior = "passthrough"
# "passthrough" : sensitive 時は addon バイパス
# "buffer"      : sensitive でも内部バッファ保持、確定時 commit
```

### 戦略切替時の状態

ユーザがフォーカスを移して戦略が変わる場合、`on_focus_change = "preserve"` であれば pending バッファは保持される。新フォーカス先の戦略に従って表示方法のみ変わる。

---

## 12. 設定パラメータ一覧

### `[settings]`

| キー | 型 | デフォルト | 説明 |
|---|---|---|---|
| `chord_window_ms` | integer | 50 | 同時シフトの BS-rewrite 受付窓（speculative 送信後の書換可能時間） |
| `mutual_window_ms` | integer | 80 | 相互シフトの BS-rewrite 受付窓 |
| `caps_lock_behavior` | enum | `"shift"` | `"shift"` / `"ignore"` / `"passthrough"` |
| `on_focus_change` | enum | `"preserve"` | `"preserve"` / `"reset"` |
| `roll_over` | bool | `true` | roll-over 許容 |
| `preedit_fallback` | enum | `"panel"` | `"client"` / `"panel"` / `"buffer"` / `"auto"` |
| `sensitive_field_behavior` | enum | `"passthrough"` | `"passthrough"` / `"buffer"` |

### `[[modifier]]`

| キー | 型 | デフォルト | 説明 |
|---|---|---|---|
| `id` | string | 必須 | 識別子 |
| `key` | string | 必須 | キー識別子 |
| `kind` | enum | `"hold"` | `"hold"` / `"oneshot"` |
| `hold_detection` | enum | `"interrupt"` | `"interrupt"` / `"timeout"` |
| `hold_timeout_ms` | integer | 150 | timeout 方式時のみ |
| `tap_action` | enum | `"send_key"` | `"send_key"` / `"none"` |

### `[[layer]]`

| キー | 型 | 説明 |
|---|---|---|
| `id` | string | 識別子 |
| `kind` | enum | `"single"` / `"prefix"` / `"postfix"` / `"modified"` |
| `trigger` | string | prefix/postfix 時のトリガキー |
| `modifier` | string | modified 時の参照 modifier id |
| `grid` | string | グリッド表記レイアウト |

### `[[chord]]`

| キー | 型 | 説明 |
|---|---|---|
| `keys` | array[string] | 同時押し対象キー |
| `output` | string | 出力かな |
| `symmetric` | bool | 相互シフトなら `true` |
| `window_ms` | integer | 設定の chord/mutual window を個別上書き |

### `[[direct]]`

| キー | 型 | 説明 |
|---|---|---|
| `sequence` | array[string] | キー列 |
| `output` | string | 直接 commit する漢字 |

### `[direct_trigger]` （optional）

| キー | 型 | デフォルト | 説明 |
|---|---|---|---|
| `keys` | array[string] | 必須 | トリガキー（複数 OR） |
| `kind` | enum | `"hold"` | `"hold"` / `"toggle"` |
| `hold_detection` | enum | `"interrupt"` | `"interrupt"` / `"timeout"`（kind = "hold" 時のみ） |
| `hold_timeout_ms` | integer | 150 | timeout 方式時のみ |
| `tap_action` | enum | `"passthrough"` | `"passthrough"` / `"none"` |

---

## 13. 実装構成

### Cargo workspace 構成

```
mzkana/
├── crates/
│   ├── mzkana-core/             # 状態機械 + 設定 + IPC（pure Rust）
│   │   ├── src/
│   │   │   ├── config.rs         # TOML パース + JSON Schema 生成
│   │   │   ├── statemachine.rs   # Pattern matching engine
│   │   │   ├── mozc/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── proto.rs      # prost 生成
│   │   │   │   └── client.rs     # UDS IPC
│   │   │   ├── reload.rs         # notify によるホットリロード
│   │   │   └── lib.rs
│   │   ├── build.rs              # protobuf + JSON Schema 自動生成
│   │   └── Cargo.toml
│   ├── mzkana-ffi/              # C ABI export（cbindgen）
│   ├── mzkana-cli/              # 設定検証 + dry-run テスト
│   └── mzkana-config-gui/       # egui ベースの設定 GUI
├── fcitx5-addon/                 # C++ 薄シム（CMake）
│   ├── src/
│   │   ├── engine.cpp            # FcitxInputMethodEngineV2 impl
│   │   └── main.cpp              # addon entry
│   └── CMakeLists.txt
├── schemas/
│   └── layout.schema.json        # build.rs で自動生成
├── layouts/                      # 同梱配列
│   ├── tsuki-2-263.toml
│   ├── shin-geta.toml
│   ├── naginata-v17.toml
│   └── t-code.toml
└── Cargo.toml
```

### 主要依存クレート

| crate | 用途 |
|---|---|
| `serde` + `toml` | 設定パース |
| `schemars` | JSON Schema 生成 |
| `prost` + `prost-build` | Mozc protobuf |
| `tokio` | 非同期 UDS IPC |
| `notify` | ファイル監視 |
| `tracing` | 構造化ログ |
| `cbindgen` | C ヘッダ生成 |
| `eframe` + `egui` | GUI バイナリ |

---

## 14. 実装フェーズ

```
Phase 1: mzkana-core 単体
  ├ TOML パース + JSON Schema 生成
  ├ State machine（全シフト方式、speculative execution + BS-rewrite）
  │   ├ tentative_buffer の管理
  │   ├ pending_keys と規則マッチング
  │   └ CONVERSION モード遷移ハンドラ
  ├ mzkana-cli で synthetic key events を流して検証
  │   └ 「キー列 → 出力イベント列（key_string / BackSpace の混在）」を検証
  └ 既存配列（月、新下駄、薙刀式、T-code）の TOML を書いて回帰テスト

Phase 2: Mozc 接続
  ├ prost-build で commands.proto 取り込み
  ├ UDS クライアント
  ├ BS-rewrite プロトコル実装（BackSpace + 新 kana 連続送信）
  └ cli で「キー列入力 → mozc 経由 preedit/result」を確認、書換動作も検証

Phase 3: fcitx5 アドオン化
  ├ C++ シム + cbindgen
  ├ KeyEvent → core 呼び出し → preedit/commit 同期
  ├ Preedit 表示戦略（ClientInline/PanelPreedit/BufferOnly）の切替
  └ ホットリロード（notify）

Phase 4: 設定 GUI
  └ egui で配列の視覚編集 + プレビュー
```

### 各フェーズの完了条件

| Phase | 完了条件 |
|---|---|
| 1 | 4 種類の既存配列が cli で正しいかな列を出力する |
| 2 | cli から実際の Mozc サーバに接続して preedit/result が得られる |
| 3 | fcitx5 上で実用入力ができ、設定リロードが動く |
| 4 | GUI で配列の編集・保存・即時プレビューができる |

---

## 付録 A. 既存配列との互換性

### 想定する配列

| 配列 | mode | 使用するシフト方式 |
|---|---|---|
| 月配列 2-263 式 | kana | 前置シフト |
| 月配列 K | kana | 前置シフト + 同時シフト |
| 新下駄配列 | kana | 同時シフト |
| 薙刀式 | kana | センターシフト + 相互シフト |
| カタナ式 | kana | 通常シフト + 後置シフト |
| T-code | kanchoku | 2 ストローク sequence |
| TUT-code | hybrid | 2 ストローク sequence + かな |
| いろは坂配列 | kana | 通常シフト |

### 既存実装との対応

| 既存ソフト | プラットフォーム | MzKana の位置付け |
|---|---|---|
| やまぶき R | Windows | Linux 版相当 |
| 紅皿 | Windows | Linux 版相当 |
| Kanata | クロス | キーボード層、MzKana は IME 層 |
| かえうち | ハードウェア | ソフトウェア代替 |
| fcitx5-anthy + 設定 | Linux | 制限を超える表現力（同時/相互シフト対応） |
| Mozc ローマ字テーブル方式 | クロス | speculative execution 機構の参考実装 |

---

## 付録 B. Speculative execution 動作トレース

主要シフト方式の挙動を、ユーザの視覚（preedit に表示される内容）と内部状態で追う。

### 月配列 2-263 式（中指前置シフト）

```
配列定義:
  base layer: 'd' 位置 = "か", 'k' 位置 = "い"
  [[layer]] kind=prefix trigger="d": [d, k] → "れ"

操作: d → k
─────────────────────────────────────────
t=0   'd' KeyDown
       speculative_emit: base 出力 "か" を Mozc に送信
       preedit: "か"
       tentative_buffer: [("か", from=[d])]
       pending_keys: [d]（[d, k] 候補が残るため保持）

t=Δ   'k' KeyDown（Δ は任意時間、無制限）
       規則 [d, k] → "れ" マッチ
       on_rule_match: BS 送信 × 1、"れ" 送信
       preedit: "れ"
       tentative_buffer: [("れ", from=[d, k])]
       pending_keys: []

操作: d 単独（その後 Space）
─────────────────────────────────────────
t=0   'd' 押下 → preedit: "か"、書換可能状態
t=∞   Space   → Mozc が CONVERSION、tentative_buffer クリア
                 "か" は確定的に変換対象になる
```

### 新下駄配列（同時シフト）

```
配列定義:
  base layer: 'f' = "は", 'j' = "と"
  [[chord]] keys=[f, j] output="を"
  chord_window_ms = 50

操作: f + j（同時押し）
─────────────────────────────────────────
t=0    'f' KeyDown
        speculative_emit: "は" 送信
        preedit: "は"
        rewrite_deadline: t + 50ms

t=20ms 'j' KeyDown（window 内）
        規則 [f, j] chord マッチ
        BS 送信、"を" 送信
        preedit: "を"

操作: f → ポーズ → j（chord 不成立）
─────────────────────────────────────────
t=0    'f' 押下 → preedit: "は"
t=50ms ChordTimer 発火、"は" 確定
t=200ms 'j' 押下 → speculative "と"
        preedit: "はと"
```

### 薙刀式（センターシフト + 相互シフト）

```
配列定義:
  [[modifier]] id="center" key="space" kind="hold" detection="interrupt"
  [[layer]] id="center_shift" kind="modified" modifier="center"
    grid: 'a' 位置 = "シフト面の a kana"
  [[chord]] keys=["a", "s"] symmetric=true output="ざ"

操作: Space hold + a（連続シフト）
─────────────────────────────────────────
t=0    Space KeyDown → modifier "center" 押下
t=10ms 'a' KeyDown
        center modifier active → center_shift layer の "a" 出力を直接送信
        speculative 不要（modified layer は最終出力が確定的）
        preedit: "シフト面 a"

t=20ms 'a' KeyUp, Space KeyUp → modifier 解除

操作: a と s の相互（順序問わず）
─────────────────────────────────────────
t=0    's' KeyDown
        speculative_emit: "s の base kana" 送信
        rewrite_deadline: t + 80ms

t=30ms 'a' KeyDown（mutual window 内）
        相互 chord [a, s] / [s, a] 両方マッチ（symmetric）
        BS、"ざ" 送信
        preedit: "ざ"
```

### T-code（2 ストローク漢直、silent）

```
配列定義:
  meta.mode = "kanchoku"
  [[direct]] sequence=["k", "j"] output="日"

操作: k → j
─────────────────────────────────────────
t=0    'k' KeyDown
        direct_trigger_active = true（kanchoku mode で常時）
        speculative なし（silent）
        preedit: "" 変化なし
        pending_keys: [k]

t=Δ    'j' KeyDown
        sequence [k, j] → "日" 完成
        Mozc に SUBMIT、漢字 "日" を直接 commit
        application: "日" が確定的に追加表示
```

### カタナ式（後置シフト）

```
配列定義:
  base layer: 'a' = "あ"
  [[layer]] kind="postfix" trigger="k": [a, k] → "ぁ"

操作: a → k
─────────────────────────────────────────
t=0    'a' KeyDown
        speculative_emit: "あ" 送信
        preedit: "あ"
        rewrite_deadline: None（sequence は永久書換可能）

t=Δ    'k' KeyDown
        規則 [a, k] postfix → "ぁ" マッチ
        BS、"ぁ" 送信
        preedit: "ぁ"

操作: a → ポーズ → b（postfix 不成立）
─────────────────────────────────────────
t=0    'a' 押下 → preedit: "あ"
t=∞    'b' 押下 → [a, b] 規則なし
                  "あ" 確定、'b' を speculative "い"
                  preedit: "あい"
```

---

## 付録 C. 未決事項・将来検討

- **AltGr / Level 3 サポート**: 現状 level 0 + Shift のみ。日本語キーボードで需要が出れば対応。
- **物理位置ベースの記述モード**: Dvorak/Colemak ユーザ向けに、XKB に依らず scancode で記述するオプションレイヤーを将来追加可能。
- **配列定義の継承**: 基底 TOML を `extends` で参照して差分のみ書くスキーマ拡張。
- **入力統計**: タイピング解析機能（既存の tsuki_optimizer と連携可）。
- **マルチプロセス対応**: Mozc セッションを複数 IME インスタンス間で共有するか分離するか。
