# MzKana 設計書 v0.3

Fcitx5 上で動作するかな配列・漢直入力エンジンの設計仕様。

> v0.3 で実装（Phase 1–3 完了時点）に合わせて全面的に整合を取った。設計思想を
> 示す概念図と、実装の事実が異なる箇所には「実装メモ／注」を付した。主な実装側の
> 確定事項: 同期 IPC + ワーカースレッド（非 tokio）、proto の vendor + prost-build、
> かな送信は AS_IS、セッション初期化は TURN_ON_IME、BS-rewrite は preedit 文字数
> ベース、オートリピート抑止は core 側。
>
> GUI 機能の章: §11.5 候補ウィンドウ（予測・変換候補、Mozc 準拠、Phase 4）は
> **Rust（コア + FFI）実装済み・単体テスト済み、C++ 部分は記述済みだが fcitx5
> 開発環境が無く未コンパイル（実機ビルド要）**。§13.5 設定 GUI（fcitx5-configtool
> 連携で配列ファイル選択、Phase 5）は設計検討のみで未実装。

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
11.5. [候補ウィンドウ（予測・変換候補）](#115-候補ウィンドウ予測変換候補の表示)
12. [設定パラメータ一覧](#12-設定パラメータ一覧)
13. [実装構成](#13-実装構成)
13.5. [設定 GUI（configtool 連携）](#135-設定-guifcitx5-configtool-連携)
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

> **実装メモ**: 本章は設計思想（各シフト方式を「制約付きキー列マッチング」に
> 還元する考え方）を示す。実際の `mzkana-core` は下記の単一 `Rule`/`Pattern` enum
> ではなく、`Layout` 内の `base_layer` / `prefix_layers` / `postfix_layers` /
> `modified_layers` / `chords` / `directs` という**個別フィールド**として保持し、
> `process_key_down` が方式ごとの分岐で評価する。概念モデルとして参照のこと。

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

### 3 キー以上の同時シフト（最長一致 + 段階的アップグレード）

`chord.keys` は 2 キーに限らず、3 キー以上の同時押し（濁拗音 `ぎゃ` = `[w,h,j]`、
外来音 `ふぁ` = `[semicolon,j,v]` 等）も表現できる。実装は別経路を持たず、
speculative + BS-rewrite 機構にそのまま乗る。

- **最長一致**: `find_mutual_chord_match` は、すべてのキーが held で `new_key` を
  含む symmetric chord のうち **最長のもの** を選ぶ（`max_by_key(keys.len())`）。
- **段階的アップグレード**: `w↓` で `き`（base）→ `h↓` で `[w,h]→きゃ`（2 キー、
  BS×1）→ `j↓` で `[w,h,j]→ぎゃ`（3 キー、BS×2 で `きゃ` を消し再送）。途中の
  `きゃ` は確定前なので BS-rewrite で `ぎゃ` に置き換わる。完全同時押し（w h j が
  すべて held）でも最長一致で `ぎゃ` が選ばれる。
- 2 キー部分集合（`[w,h]→きゃ`）と 3 キー（`[w,h,j]→ぎゃ`）は共存でき、`validate`
  でも競合扱いしない（最長一致で解決）。

---

## 4. キー識別子

### XKB の keysym を直接利用する

fcitx5 が addon に渡す KeyEvent は既に XKB 変換後の keysym を持っている。これをそのまま識別子化する。専用の scancode → 文字テーブルは不要。

**実装（`fcitx5-addon/src/engine.cpp`）**: `key.sym()`（解決済み keysym）を使い、
A–Z（Shift 付き英字）のみ算術的に小文字へ正規化してから `keySymToString` で
識別子化する。Shift フラグは別途 core に渡す。Ctrl/Alt/Super/Hyper 同時押しと
bare modifier（Shift_L 等）は評価前に除外する。

```cpp
// fcitx5-addon: keyEvent
if (key.states() & {Ctrl, Alt, Super, Hyper}) return;  // 修飾付きはアプリへ
if (key.isModifier()) return;                          // bare modifier は無視
bool shift = key.states() & KeyState::Shift;

fcitx::KeySym sym = key.sym();
if (sym >= FcitxKey_A && sym <= FcitxKey_Z)            // A–Z → a–z
    sym = lower(sym);
std::string keyName = fcitx::Key::keySymToString(sym); // "a","q","comma","yen"…
// → mzkana_engine_key_down/up(engine, keyName, shift)
```

> 注: 設計初稿は `xkb_state_key_get_one_sym_for_level(..., level 0)` による完全な
> level-0 取得を想定していたが、実装は A–Z の算術小文字化のみ。記号・JIS 固有キーの
> Shift 変化（数字段の記号等）は現状吸収しきれない（§付録 C / 既知の制限）。

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

OS は物理キー保持中に同じ key-down を連打する（オートリピート）。状態機械は
押下/解放の離散遷移を前提とするため、保持中キーの再 down は誤爆の原因になる。
**抑止は Rust core 側で行う**: `process_key_down` 冒頭で、既に `held_keys` にある
通常キーの down を無視する。BackSpace と Mozc 制御キー（Return/矢印/Henkan 等）は
`held_keys` に積まないため、それらのリピート（連続削除・候補送り）は有効なまま。
（設計初稿は C++ 側 `is_repeat` での抑止を想定していたが、実装は core 側に集約。）

---

## 5. 状態機械アルゴリズム

### 内部状態

実際のフィールド名は `statemachine.rs` に準拠（下記は要点の抜粋）。

```rust
struct StateMachine {
    layout: Layout,                          // コンパイル済み配列（base_layer/chords/… 別フィールド）
    pending_keys: Vec<(String, Instant)>,
    tentative_buffer: Vec<TentativeChar>,    // Mozc に送ったが書換可能な文字
    modifier_states: Vec<ModifierState>,     // modifier ごとの hold/toggle/oneshot 状態
    direct_trigger_state: DirectTriggerState,// 7 章の漢直トリガー状態
    mozc_mode: MozcMode,                     // Composition | Conversion
    chord_deadline: Option<Instant>,         // tentative_buffer 内の最早 rewrite_deadline
    held_keys: HashSet<String>,              // 物理押下中の通常キー（相互/同時シフト判定 + リピート抑止）
    chord_consumed_keys: HashSet<String>,    // 確定済み chord のキー（解放まで再発火を抑止）
}

struct TentativeChar {
    kana: String,             // Mozc preedit に送った文字（合成済みかな・複数文字もあり得る）
    source_keys: Vec<String>, // どのキー（列）から生成したか
    rewrite_deadline: Option<Instant>,  // chord 候補がある場合のみ Some
    pending_tail: Vec<String>, // 確定後に emit する後続トークン（"、!Enter" 等の末尾）
    mozc_char_len: usize,      // この kana が Mozc preedit で占める文字数（H1: BS 回数の基準）
    confirmed: bool,           // source キーのいずれかが解放されたら true（以後 chord 書換対象外）
}
```

> 注: 設計初稿の `sent_at` / `chord_timer` / `rules: RuleIndex` は実装には存在しない。
> 規則は `Layout` 内の `base_layer` / `prefix_layers` / `postfix_layers` /
> `modified_layers` / `chords` / `directs` / `aliases` という個別フィールドで保持し、
> 都度線形マッチする（§3 の統一 `Rule`/`Pattern` 抽象は採用していない。下記参照）。

### 核心：Speculative execution + BS-rewrite

未確定中のキーは常に「最善推定」の kana を Mozc preedit に即時送信し、より優先される規則が後からマッチしたら BS で書き換える。これにより、シフト方式に関わらずユーザは即時の視覚フィードバックを得つつ、任意の時間後の解決を許容できる。

### イベント処理

```
KeyDown(k, t):
  if k が既に held_keys にある（= OS のオートリピート）and k が通常キー:
    return   # 押下/解放の離散遷移のみを扱う。保持中キーの down 連打は無視
             # （BackSpace・制御キーは held_keys に積まないのでリピートは有効）

  if k は modifier 定義に該当:
    active_modifiers.insert(modifier_id)
    return

  if k は direct_trigger.keys に該当:
    direct_trigger_active を更新（kind に応じて hold/toggle）
    pending_keys と tentative_buffer をクリア
    return

  if mozc_mode == CONVERSION and k が Mozc 制御キーでない:
    # 通常キーは新規合成を開始（書換機会消失、Composition へ戻す）
    tentative_buffer.clear(); pending_keys.clear(); mozc_mode = Composition
    # ※ space/Henkan/矢印/Return/Escape 等の制御キーはこの分岐に入らず、
    #   変換を継続するため後段で Mozc へ送られる（候補送り・確定が効く）

  # 相互シフト（symmetric chord）は held_keys が揃った時点で即発火（後述）
  pending_keys.push((k, t))
  speculative_emit(k)   # base 割当があれば即 speculative 送信、なければ C4 経路へ

KeyUp(k, t):
  held_keys.remove(k); chord_consumed_keys.remove(k)
  # このキーを source に持つ tentative char を confirmed=true にマーク
  #   → 以後の chord 書換から保護（あかが 問題の核心）
  if k は modifier/direct_trigger:
    その tap_action を評価（hold 解除 / 単独タップ出力）
    return

tick(now):   # fcitx5 の EventSourceTime（約10ms 周期）から呼ばれる
  chord_deadline 超過分の tentative を確定し pending_tail を emit
  window 超過の pending_keys を除去（chord_window の時間強制, C1）
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
    # base 層に割当なし。chord 候補もなければ pending から外し、
    # 合成状態（tentative 非空 or Conversion）に応じて経路分岐（C4）:
    #   合成中 + Mozc 制御キー(space/Return/矢印/Henkan 等) → SendFunctionKey で Mozc へ
    #   合成中 + 非制御キー → SubmitThenPassthrough（preedit を確定→生キー素通し）
    #   非合成 → Passthrough（アプリへ素通し）
    return

  send_to_mozc(KeyString(base.kana))   # AS_IS で preedit へ挿入
  tentative_buffer.push(TentativeChar {
    kana: base.kana,
    source_keys: vec![k],
    mozc_char_len: base.kana.chars().count(),
    confirmed: false,
    rewrite_deadline:
      if chord 候補あり: Some(t + chord_window_ms)
      else if sequence 候補あり: None  # sequence は永久書換可能
      else: 即時 None（書換機会なし、確定済み）
  })

  # sequence/chord の継続候補があれば pending_keys に保持、なければ clear
```

### 規則マッチ時の BS-rewrite

```
on_rule_match(rule):
  # tentative_buffer 末尾から、書換対象の TentativeChar 群を選ぶ。
  # 条件: !confirmed かつ source_keys が全て rule のキー集合に含まれる（部分集合）。
  #   - confirmed（source キー解放済み）の文字は確定とみなし絶対に巻き込まない。
  #     例: あ(j) か(f) を打った後に が(f+j) を打つと、確定済み「あ」「か」の
  #         source [j] [f] は {f,j} の部分集合だが confirmed なので除外される。
  affected = tentative_buffer.rev().take_while(|tc|
               !tc.confirmed && tc.source_keys ⊆ rule.keys)

  # BackSpace は「対象文字の mozc_char_len の合計」回送る（H1: 1 TentativeChar が
  # 複数文字になり得るため、文字数ベースで Mozc preedit と同期させる）。
  send_to_mozc(BackSpace × Σ affected.mozc_char_len)
  tentative_buffer.truncate(affected を除いた残り)

  # rule の出力を新たに送信
  send_to_mozc(KeyString(rule.output))
  tentative_buffer.push(TentativeChar { kana: rule.output, source_keys: rule.keys, … })

  pending_keys.clear()
```

> **H1 不変条件**: BackSpace 回数は TentativeChar 数ではなく Mozc preedit 文字数。
> 削除＋BackSpace 生成＋truncate は `pop_tentative(n)` に集約され、全書換経路
> （chord / prefix / 後置 / kanchoku 遷移 / 手動 BS）がこれを共有する。

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
    末尾の TentativeChar を 1 件 pop
    その mozc_char_len の回数だけ BackSpace を Mozc へ送る（複数文字かな対応）
    pending_keys.clear()
  else if pending_keys 非空:
    pending_keys.clear()  # 未完成 prefix/sequence を破棄（外部 BS は送らない）
  else:
    BackSpace をアプリへ素通し（内部に何も無い）
```

BackSpace は `held_keys` に積まないため、押しっぱなしのオートリピートによる
連続削除はそのまま機能する（KeyDown 冒頭のリピート抑止対象外）。

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

Modifier 系の出力（modified layer の kana）は最初から最終層の kana を送るため、書換は発生しない。

センターシフトキー（薙刀式の `space` 等、`tap_action = "send_key"`/passthrough）の
**単独タップ**は、その物理キーが Mozc 制御キー（space/Return 等）なら合成中・非合成を
問わず Mozc へ送る（`SendFunctionKey`）。これにより空 preedit 時の space は全角「　」を
commit、合成中の space は変換を開始する。非制御キーの passthrough modifier はそのまま
アプリへ素通しする。

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
key             = "space"        # 複数キーも可: key = ["space", "henkan"]
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

公式 `commands.proto`（および推移的 import: `config.proto` / `candidate_window.proto`
/ `engine_builder.proto` / `user_dictionary_storage.proto`）を
`crates/mzkana-core/protocol/` に **vendor**（リポジトリにコミット）し、
`prost-build` + `protoc-bin-vendored` でビルド時に Rust 型を生成する
（システム protoc 不要・ネットワーク不要、`crates/mzkana-core/build.rs`）。

### 接続先

Linux 抽象名前空間ソケット（`/proc/net/unix` から
`@…​.mozc.….session` を自動検出）を優先。検出できない場合のフォールバックとして
`~/.mozc/session.sock`（filesystem socket）を使う。認証はカーネルの SO_PEERCRED
による UID 照合で、鍵交換は行わない。未起動なら既知パスから mozc_server を自動起動する。

### スレッドモデル（同期 IPC + ワーカースレッド）

`MozcClient` は同期ブロッキングの `std::os::unix::net::UnixStream` を用いる
（tokio 等の非同期ランタイムは使わない）。fcitx5 のメインイベントループを塞がない
よう、IPC は専用ワーカースレッド `MozcWorker`（`mozc/worker.rs`）上で実行する。

- engine ↔ worker は `std::sync::mpsc` チャネルで通信。
- 1 打鍵分の操作（BS-rewrite の BackSpace×N + SendKana 等）は `Op` 列として
  **1 バッチ**にまとめ、チャネル往復とタイムアウト予算を 1 回に集約する。
- `recv_timeout` による **150ms ハードタイムアウト**。超過時はワーカーを
  sticky-dead 化して破棄し、当該キーは未確定のまま素通し、次イベントで再接続する
  （UI は決して固まらない）。ソケット自体の read/write タイムアウトは 1 秒。

### キー送信の方式

ローマ字としてではなく、**確定したかな文字列を直接送る**。`input_style` は
**`AS_IS`**（値 1）を使う。Mozc 内部の `InsertCharacterPreedit()` が呼ばれ、
かな文字列がローマ字変換テーブルを介さず composition（preedit）へ直接挿入される。

```rust
// proto.rs: input_send_kana
let key = KeyEvent {
    key_string: Some("か".to_string()),
    input_style: Some(input_style::AS_IS), // = 1
    ..Default::default()
};
```

`FOLLOW_MODE`(0) はローマ字テーブル経由となりかな文字列では出力されず、
`DIRECT_INPUT`(2) は preedit を介さず result へ直接 commit してしまうため、
いずれも不適。これは実 mozc_server で検証済み。

### セッション初期化（HIRAGANA モード）

CREATE_SESSION 直後のセッションは IME-OFF（Direct）状態で、この状態では
`key_string` が composer に入らない。`create_session` は session_id 取得直後に
**TURN_ON_IME**（`SessionCommand` type=22、`composition_mode = HIRAGANA`）を送って
IME を起動しつつ合成モードを設定する。`SWITCH_COMPOSITION_MODE` 単独では IME-OFF
から抜けられないことを実機で確認済みのため、TURN_ON_IME を用いる。

### BS-rewrite プロトコル

5 章の speculative execution が要求する書換動作は、Mozc IPC では
「BackSpace + 新 kana 送信」のシーケンスで実現する。BackSpace は特殊キー
（`SpecialKey::BACKSPACE`、`input_style = FOLLOW_MODE`）として送る。

```rust
fn rewrite_tentative(&self, removed_chars: usize, new_kana: &str) {
    // 1. 取消したい preedit 文字数分だけ BackSpace を送る
    for _ in 0..removed_chars {
        mozc.send_backspace()?;       // SpecialKey::BACKSPACE
    }
    // 2. 新しい kana を送る
    mozc.send_kana(new_kana)?;        // key_string + AS_IS
}
```

AS_IS で挿入された kana は Mozc preedit の末尾に追加され、BackSpace は末尾の
1 文字を削除する。我々の `tentative_buffer` と Mozc preedit が一対一で対応する
不変条件が保てる。ただし 1 つの `TentativeChar` が複数文字（"きゃ"・合成かな・
alias 展開）になり得るため、BackSpace 回数は TentativeChar 数ではなく
**Mozc preedit 文字数**（`TentativeChar.mozc_char_len` の合計）で数える（5 章 H1）。

### preedit / commit 同期

```
Mozc.Output.preedit    → fcitx5 の preedit 表示に反映（11 章の戦略に従う）
Mozc.Output.result     → fcitx5 経由で commit
Mozc.Output.candidates → fcitx5 の候補ウィンドウに反映
Mozc.Output.mode       → 状態機械の mozc_mode を更新
```

### CONVERSION モード遷移の検出

ユーザが Space 等を押して変換候補を開いた時、Mozc は `Output.mode = CONVERSION` を返す。状態機械側は以下を行う：

FFI 層（engine.rs）は Mozc Output の preedit highlight 等から CONVERSION 遷移を
判定し、`StateMachine::notify_mozc_conversion()` を呼ぶ。状態機械側の処理:

```rust
fn notify_mozc_conversion(&mut self) {
    if self.mozc_mode != Conversion {
        self.tentative_buffer.clear();   // 書換機会消失
        self.pending_keys.clear();
        self.held_keys.clear();          // 新入力サイクル: 滞留キー/誤リピート判定を防止
        self.chord_consumed_keys.clear();
    }
    self.mozc_mode = Conversion;
}
```

CONVERSION 中は新たな kana キー入力で Mozc が自動的に COMPOSITION に戻る（候補確定 + 新規入力）ため、特殊な復帰処理は不要。

### 漢直 commit との相互作用

```rust
fn handle_direct_output(kanji: &str) {
    // 1. tentative_buffer が残っていれば全て BS で消す（preedit 文字数ベース）
    let bs = self.flush_tentative();   // tentative_buffer をクリアし BackSpace 群を返す
    // 2. Mozc preedit を確定（残っていれば変換した上で commit される）→ submit
    // 3. 漢直結果を直接 commit（fcitx5 の commitString）
    // 4. 状態リセット（pending_keys クリア）
    //
    // 実装では state machine が SubmitAndCommit(kanji) / CommitDirect(kanji) 等の
    // OutputAction を返し、FFI 層（engine.rs）がワーカー経由で submit→commit を行う。
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

> **実装メモ**: 本章は競合検出の完全な設計（context / activation_scope モデル）を
> 示す。実装の `config::analyze_conflicts` は現状その部分集合をカバーする:
> ① 多重ロールキー（同一キーが modifier / direct trigger / prefix / postfix を兼ねる）、
> ② base 層を覆う特殊キー、③ 出力の異なる重複 chord、④ modifier に飲まれる chord メンバ。
> direct の完全重複（出力相違）は読込時にハード error。これらは `load_layout` が
> tracing 警告として出力し、`mzkana-cli validate` が件数と内容を表示する。
> fcitx5 通知（notification）連携と activation_scope ベースの厳密判定は未実装。

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

### 4 段階の戦略（概念）と実装

概念上は次の 4 戦略を想定する。

```rust
enum PreeditStrategy {
    ClientInline,         // CapabilityFlag::Preedit あり、inline 表示
    PanelPreedit,         // inline 不可、変換窓内に表示
    BufferOnly,           // パネル描画も不可、内部バッファのみ、確定時に commit
    PassthroughImmediate, // sensitive、addon バイパス
}
```

> **実装メモ**: C++ 側に上記 enum は無く、`applyResult`（`engine.cpp`）が
> capability と `preedit_fallback` 設定で**インラインに分岐**する:
> - `CapabilityFlag::Preedit` 対応クライアント → `setClientPreedit`（インライン）。
>   併せて panel にもミラーする。
> - 非対応 かつ `preedit_fallback = "buffer"` → client/panel とも空（非表示）。
> - それ以外（client/panel/auto）→ フローティングパネルに表示。
>
> 実効的に ClientInline / PanelPreedit / BufferOnly を区別する。`PassthroughImmediate`
> は別経路で実装: パスワード/sensitive 欄では `sensitive_field_behavior` に従い、
> `passthrough` なら IME 処理自体をスキップ、`buffer` なら処理するが preedit は出さない
> （後述「sensitive フィールドの扱い」）。"client" と "auto" は現状パネルと同等に扱う。

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

## 11.5. 候補ウィンドウ（予測・変換候補の表示）

> Phase 4。Rust（コア + FFI）実装済み・単体テスト済み、C++ 部分は記述済みだが
> fcitx5 開発環境が無く未コンパイル（実機ビルド要）。Mozc が返す候補を fcitx5 の Input Panel に
> 表示し、変換動作は可能な限り Mozc に倣う。
### 背景と現状

`Output.candidate_window`（protobuf tag 6）には予測・変換候補が含まれるが、現状の
`decode_response` はこれを読んでおらず、候補は一切表示していない。本章で 2 種類の
候補表示を設計する。

| 種別 | 契機 | 表示 |
|---|---|---|
| 予測候補（suggestion） | かな入力中（合成中）に Mozc が suggestion を返す | preedit の下に区切って一覧 |
| 変換候補（conversion） | Space 等で変換開始（`focused_index` あり） | Mozc 風の縦型候補ウィンドウ |

両者は Mozc 上は同じ `CandidateWindow` メッセージで表現され、`focused_index` の
有無で区別される（has_focused_index ⇒ 変換、無 ⇒ suggestion）。

### データフロー

```
mozc_server
  │  Output { preedit, candidate_window{ focused_index?, size, candidate[], position }, … }
  ▼
mozc/proto.rs : decode_response  … candidate_window をデコードして DecodedOutput に追加
  ▼
mozc/mod.rs   : MozcOutput.candidates: Vec<Candidate>, focused_index: Option<u32>
  ▼
ffi/engine.rs : ProcessResult に候補を載せる（可変長のため別 FFI 関数で取得）
  ▼
fcitx5-addon  : CommonCandidateList を構築し inputPanel().setCandidateList()
```

### コア側（mozc-core）

`DecodedOutput` / `MozcOutput` に候補フィールドを追加する。

```rust
pub struct Candidate {
    pub index: u32,
    pub value: String,
    pub id: Option<i32>,         // SELECT/HIGHLIGHT_CANDIDATE 用の Mozc 内部 id。
                                 // None は「id 未割当＝選択不可」。FFI では -1 で表現し、
                                 // C++ / select_candidate は負 id を no-op として無視する。
    pub annotation: Option<String>, // 注釈（[半][カナ] 等、description/suffix）
}

pub struct MozcOutput {
    // 既存: preedit, result, is_converting, mode, consumed
    pub candidates: Vec<Candidate>,
    pub focused_index: Option<u32>, // Some ⇒ 変換中（縦型窓）、None ⇒ suggestion
    pub candidate_size: u32,        // 総候補数（ページング用、candidate[] は現ページ分のみ）
}
```

`decode_response` で `Output.candidate_window` を読み、`candidate[]`（group, tag 3）の
`index`(4) / `value`(5) / `id`(9) / `annotation`(7) を取り出す。prost 生成型で
そのまま辿れる（`output.candidate_window.candidate` 等）。

### FFI 境界

`MzkanaResult` は固定長フラット構造のため可変長の候補リストを載せられない。
候補は**別関数**で取得する（key_event 後に C++ が呼ぶ）。

```c
typedef struct { const uint8_t* value; uint32_t value_len;
                 const uint8_t* annotation; uint32_t annotation_len;
                 int32_t id; } MzkanaCandidate;

// 直近の Mozc 出力の候補数（focused 時は変換、それ以外は suggestion）
uint32_t mzkana_engine_candidate_count(const MzkanaEngine* e);
// i 番目の候補を取得（value/annotation はエンジン所有、次の key_event まで有効）
MzkanaCandidate mzkana_engine_candidate(const MzkanaEngine* e, uint32_t i);
// 変換中フォーカス位置（変換中のみ）。未変換/予測時は -1。
int32_t mzkana_engine_focused_index(const MzkanaEngine* e);
```

エンジンは直近の `MozcOutput` を保持し、候補文字列のバッファ所有権を握る
（次キーイベントで上書き）。これにより `MzkanaResult` の ABI 互換を壊さない。

### C++ 側（fcitx5-addon）

`applyResult` の後段で候補を反映する。

```cpp
void MzkanaFcitxEngine::applyCandidates(fcitx::InputContext *ic) {
    uint32_t n = mzkana_engine_candidate_count(engine_);
    if (n == 0) { ic->inputPanel().setCandidateList(nullptr); return; }

    auto list = std::make_unique<fcitx::CommonCandidateList>();
    list->setPageSize(9);
    list->setSelectionKey(fcitx::Key::keyListFromString("1 2 3 4 5 6 7 8 9"));
    list->setLayoutHint(fcitx::CandidateLayoutHint::Vertical); // Mozc 風縦型
    for (uint32_t i = 0; i < n; ++i) {
        auto c = mzkana_engine_candidate(engine_, i);
        list->append<MzkanaCandidate>(c.id, toStr(c.value), toStr(c.annotation));
    }
    int focused = mzkana_engine_focused_index(engine_);
    if (focused >= 0) list->setGlobalCursorIndex(focused); // 変換時はハイライト
    ic->inputPanel().setCandidateList(std::move(list));
    ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
}
```

- **予測候補**: preedit の下に区切って表示したい。fcitx5 には preedit と候補の間の
  「区切り線」専用 API は無く、classicui がテーマに従って preedit→候補を縦に積む。
  Mozc 風のヘッダ（「Tab で予測」等）が要るなら `inputPanel().setAuxDown(Text)` に
  文字列を置く。最小実装では候補リストをそのまま出せば preedit 直下に並ぶ。
- **変換候補**: `setLayoutHint(Vertical)` + `setGlobalCursorIndex(focused)` で
  Mozc の縦型・ハイライト付き窓を再現。注釈（`annotation`）は候補の comment として
  表示できる（`CandidateWord::setComment`）。

### 変換操作のキー経路（Mozc 準拠）

候補選択ロジックは**自前で持たず Mozc に委ねる**。これが Mozc 準拠の最短路。

```
変換中（candidate list がアクティブ）の keyEvent:
  Space / Down       → Mozc へ送る（次候補へフォーカス移動）。返ってきた
                        focused_index で setGlobalCursorIndex 更新
  Up                 → Mozc へ（前候補）
  数字 1..9          → その表示位置の候補を SELECT_CANDIDATE（id 指定）で確定
  Enter              → 現フォーカス候補を確定（SUBMIT / SUBMIT_CANDIDATE）
  Escape             → REVERT（変換取消、合成に戻る）
  その他の文字キー   → 現候補を確定し、新規入力として処理（§5 の Conversion 分岐）
```

数字キーによる直接選択のみ `SessionCommand::SELECT_CANDIDATE`（type 3、候補 id 指定）
を使う。Space/矢印は通常の SEND_KEY 転送で Mozc 内部のフォーカスが動き、返却された
`candidate_window.focused_index` に UI を追従させる。これにより候補の並び・ページング・
確定挙動が Mozc 本体と一致する。

### 設定

```toml
[settings]
candidate_page_size  = 9       # 1 ページの候補数（既定 9）
show_prediction      = true    # 合成中の予測候補を表示するか
```

> 実装メモ: ページング（`ConvertNextPage`=20 / `ConvertPrevPage`=21）も Mozc へ
> 転送し、`candidate_window` の再取得で表示更新する。候補注釈の表示有無や
> ショートカット表記は将来の設定項目候補。

---


## 12. 設定パラメータ一覧

### `[settings]`

| キー | 型 | デフォルト | 説明 |
|---|---|---|---|
| `chord_window_ms` | integer | 50 | 同時シフトの BS-rewrite 受付窓（speculative 送信後の書換可能時間） |
| `mutual_window_ms` | integer | 80 | 相互シフトの BS-rewrite 受付窓 |
| `caps_lock_behavior` | enum | `"shift"` | `"shift"` / `"ignore"` / `"passthrough"`（※現状未強制：C++ が常に level-0 小文字化。予約） |
| `on_focus_change` | enum | `"preserve"` | `"preserve"` / `"reset"`（reset 時は focus-out で revert + preedit クリア） |
| `roll_over` | bool | `true` | roll-over 許容（※現状は別モードとして強制せず。予約） |
| `preedit_fallback` | enum | `"panel"` | `"client"` / `"panel"` / `"buffer"` / `"auto"`（buffer のみ特別扱い、他はパネル相当） |
| `sensitive_field_behavior` | enum | `"passthrough"` | `"passthrough"` / `"buffer"` |

### `[[modifier]]`

| キー | 型 | デフォルト | 説明 |
|---|---|---|---|
| `id` | string | 必須 | 識別子 |
| `key` | string \| array[string] | 必須 | 起動キー。単一（`"space"`）でも複数（`["space", "henkan"]`）でも可。複数指定時はいずれのキーでも同じ modifier が起動する。値は §4 の識別子（C++ の `keySymToString(key.sym())` を小文字化した形）。**bare modifier（`Shift_L` / `Control_L` 等）は C++ 側の `key.isModifier()` で除外されコアに届かないため起動キーに使えない**。空文字・空配列は読込時にエラー |
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
│   │   │   ├── config.rs         # TOML パース + JSON Schema 派生 + 競合解析
│   │   │   ├── statemachine.rs   # シフト方式の状態機械 + speculative/BS-rewrite
│   │   │   ├── error.rs
│   │   │   ├── mozc/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── proto.rs      # prost 生成型のラッパ + エンコード/デコード
│   │   │   │   ├── client.rs     # 同期 UDS IPC クライアント
│   │   │   │   └── worker.rs     # IPC 専用ワーカースレッド（150ms タイムアウト）
│   │   │   ├── tests.rs
│   │   │   └── lib.rs
│   │   ├── protocol/             # vendor した Mozc *.proto（commands ほか）
│   │   ├── build.rs              # prost-build + protoc-bin-vendored で proto 生成
│   │   └── Cargo.toml
│   ├── mzkana-ffi/              # C ABI export（cbindgen）+ engine/ホットリロード
│   │   ├── src/{lib.rs, engine.rs}
│   │   └── include/mzkana.h      # cbindgen 生成ヘッダ
│   └── mzkana-cli/              # 設定検証（validate）+ dry-run（run/mozc-run）+ schema 出力
├── fcitx5-addon/                 # C++ 薄シム（CMake）
│   ├── src/{engine.cpp, engine.h}
│   └── CMakeLists.txt
├── layouts/                      # 同梱配列
│   ├── tsuki-2-263.toml
│   ├── shin-geta.toml
│   ├── naginata-v17.toml
│   └── t-code.toml
└── Cargo.toml
```

> 注: ホットリロードは独立した `reload.rs` ではなく `mzkana-ffi/src/engine.rs` 内
> （`notify` watcher）。JSON Schema は build.rs ではなく `mzkana-cli schema`
> サブコマンドで出力（`schemas/` ディレクトリは持たない）。設定 GUI
> （`mzkana-config-gui` / egui）は未実装。

### 主要依存クレート

| crate | 用途 |
|---|---|
| `serde` + `toml` | 設定パース |
| `schemars` | JSON Schema 派生 |
| `prost` + `prost-build` + `protoc-bin-vendored` | Mozc protobuf（ビルド時生成・自己完結） |
| `phf` | キー集合の静的ルックアップ |
| `notify` | ファイル監視（ホットリロード） |
| `tracing` | 構造化ログ |
| `cbindgen` | C ヘッダ生成 |

> 注: IPC は同期実装でワーカースレッド分離のため `tokio` は不使用。GUI 未実装のため
> `eframe`/`egui` も不使用（設計初稿の記載を削除）。

---

## 13.5. 設定 GUI（fcitx5-configtool 連携）

> 設計検討（未実装、Phase 5 で実装予定）。独立した egui アプリ（旧 §13 の
> `mzkana-config-gui`）は作らず、**fcitx5-configtool から開く標準の設定画面**として
> 実装する。配列そのものの GUI 編集は行わない。

### 方針

- fcitx5 addon が `fcitx::Configuration` を公開し、configtool が自動で設定 UI を生成する。
- 当面の設定項目は **配列ファイルの選択（ドロップダウン）** のみ。他項目は今後追加。
- 配列ファイルの中身（グリッド・chord 等）の GUI 編集はスコープ外（TOML を直接編集）。

### 配列ファイルのドロップダウン

`~/.config/fcitx5/conf/mzkana/` 配下の `*.toml` を実行時に走査し、ファイル名の
ドロップダウンとして提示する。fcitx5 に汎用ファイルピッカー注釈は無いため、
**`EnumAnnotation` のサブクラス**で実行時にファイル一覧を列挙する（fcitx5-rime が
スキーマ一覧を出すのと同じ idiom）。

```cpp
// configtool に「実行時に決まる文字列の列挙」を伝える注釈
struct LayoutFileAnnotation : public fcitx::EnumAnnotation {
    void dumpDescription(fcitx::RawConfig &config) const {
        fcitx::EnumAnnotation::dumpDescription(config);   // IsEnum=True
        int i = 0;
        for (const auto &name : listLayoutTomlFiles()) {  // mzkana conf ディレクトリを走査
            config.setValueByPath("Enum/" + std::to_string(i), name);
            config.setValueByPath("EnumI18n/" + std::to_string(i), name);
            ++i;
        }
    }
};

FCITX_CONFIGURATION(MzkanaConfig,
    fcitx::OptionWithAnnotation<std::string, LayoutFileAnnotation>
        layout{this, "Layout", _("配列ファイル"), "naginata-v17.toml"};
    // 今後の項目はここに追加（chord_window_ms 等を Option<int> で公開予定）
);
```

### エンジン側の配線

```cpp
class MzkanaFcitxEngine : public fcitx::InputMethodEngineV2 {
    MzkanaConfig config_;
    const fcitx::Configuration *getConfig() const override { return &config_; }
    void setConfig(const fcitx::RawConfig &raw) override {
        config_.load(raw, true);
        fcitx::safeSaveAsIni(config_, "conf/mzkana.conf");
        reloadSelectedLayout();   // 選択された .toml を即座に読み込み直す
    }
    void reloadConfig() override {
        fcitx::readAsIni(config_, "conf/mzkana.conf");
        reloadSelectedLayout();
    }
};
```

- `reloadSelectedLayout()` は `config_.layout` のファイル名から実パスを解決し、
  既存の `mzkana_engine_create` / リロード経路で配列を差し替える（§10 と同じく
  Mozc preedit を revert してから差し替え）。
- 設定値は `~/.config/fcitx5/conf/mzkana.conf` に永続化される。

### .conf の変更

addon 登録ファイル `fcitx5-addon/data/mzkana.conf` で設定可能にする。

```ini
[Addon]
...
Configurable=True       # ← False から変更。configtool に「設定」ボタンが出る
```

InputMethod 登録ファイル（`mzkana-im.conf`）側は従来どおり（`Configurable` は
addon 側で指定）。

### 適用フロー

```
configtool で配列を選択 → Apply
  → fcitx5 が engine.setConfig(RawConfig) を呼ぶ
  → config_ に反映・.conf 保存・reloadSelectedLayout()
  → 以降の入力は新しい配列で動作（再起動不要）
```

ファイルを直接編集した場合の自動リロード（§10、notify 監視）と、configtool からの
選択（setConfig 経由）の 2 経路が共存する。

### 今後の設定項目（候補）

| 項目 | 型 | 備考 |
|---|---|---|
| `chord_window_ms` / `mutual_window_ms` | int | §12 の設定を GUI からも変更可能に |
| `preedit_fallback` | enum | client/panel/buffer/auto |
| `show_prediction` / `candidate_page_size` | bool/int | §11.5 候補表示 |

> これらは TOML の `[settings]` と二重管理になり得るため、「configtool の値を
> 既定とし、配列ファイルの `[settings]` で上書き可能」等の優先順位を実装時に決める
> （現時点では未決）。

---

## 14. 実装フェーズ

```
Phase 1: mzkana-core 単体  … 完了
  ├ TOML パース + JSON Schema 派生（cli schema）
  ├ State machine（全シフト方式、speculative execution + BS-rewrite）
  │   ├ tentative_buffer の管理（mozc_char_len / confirmed フラグ）
  │   ├ pending_keys / held_keys と規則マッチング、オートリピート抑止
  │   └ CONVERSION モード遷移ハンドラ
  ├ mzkana-cli で synthetic key events を流して検証（run）
  └ 既存配列（月、新下駄、薙刀式、T-code）の TOML で回帰テスト

Phase 2: Mozc 接続  … 完了
  ├ vendor した commands.proto を prost-build で取り込み（protoc 同梱）
  ├ 同期 UDS クライアント + ワーカースレッド（150ms タイムアウト）
  ├ TURN_ON_IME による HIRAGANA 初期化、AS_IS でのかな送信
  ├ BS-rewrite プロトコル（BackSpace×文字数 + 新 kana）
  └ cli mozc-run で preedit/result と書換動作を確認

Phase 3: fcitx5 アドオン化  … 完了
  ├ C++ シム + cbindgen
  ├ KeyEvent → core 呼び出し → preedit/commit 同期
  ├ tick の EventSourceTime タイマ配線（chord 確定・複合出力末尾）
  ├ Preedit 表示分岐（インライン / パネル / buffer）+ sensitive 欄処理
  └ ホットリロード（notify、engine.rs 内）

Phase 4: 候補ウィンドウ（§11.5）  … Rust 実装済み / C++ は実機検証待ち
  ├ decode_response で candidate_window をデコード（MozcOutput に candidates /
  │   focused_index / candidate_size を追加）✓ 単体テスト済み
  ├ FFI: mzkana_engine_candidate_count / _candidate / _focused_index /
  │   _select_candidate（SELECT_CANDIDATE=3 を worker 経由で送信）✓
  ├ C++: CommonCandidateList で予測（preedit 下）・変換（縦型 + focused 強調）を描画。
  │   数字キーは直接選択、Space/矢印/Enter/Esc は Mozc へ転送し focused_index に追従。
  │   ※ fcitx5 開発環境が無く当環境ではコンパイル未検証（実機ビルド要）
  └ 候補文字列はエンジンが NUL 終端バッファで保持し次キーイベントまで有効

Phase 5: 設定 GUI（§13.5、configtool 連携）  … 未着手
  ├ fcitx::Configuration + EnumAnnotation で配列ファイルのドロップダウン
  ├ addon .conf を Configurable=True に、getConfig/setConfig/reloadConfig 実装
  └ 選択即時反映（setConfig → reloadSelectedLayout）。他設定項目は順次追加
```

### 各フェーズの状況

| Phase | 完了条件 | 状況 |
|---|---|---|
| 1 | 4 種類の既存配列が cli で正しいかな列を出力する | ✅ 完了 |
| 2 | cli から実際の Mozc サーバに接続して preedit/result が得られる | ✅ 完了 |
| 3 | fcitx5 上で実用入力ができ、設定リロードが動く | ✅ 完了（実機動作確認済み） |
| 4 | 予測・変換候補が Mozc 準拠で表示され、変換操作ができる | Rust 実装済み（テスト済み）/ C++ 実機ビルド要 |
| 5 | configtool から配列ファイルを選択・即時反映できる | 未着手 |

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
