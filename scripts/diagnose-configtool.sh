#!/usr/bin/env bash
#
# diagnose-configtool.sh — fcitx5-configtool に MzKana の「設定（歯車）ボタン」が
# 出てこないときの原因切り分けスクリプト。
#
# configtool の歯車は、稼働中の fcitx5 が DBus 経由で返す
# addon / input-method のメタデータ（特に Configurable フラグ）で決まる。
# このメタデータは fcitx5 起動時に *.conf を一度だけ走査して作られるため、
#   1) Configurable=True を持つ .conf が正しい場所に入っているか
#   2) 別の場所に古い .conf が残って shadow していないか
#   3) インストール後に fcitx5 を再起動したか
# の 3 点を確認すれば原因が分かる。
#
# 使い方:  bash scripts/diagnose-configtool.sh
set -u

ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*"; }
bad()  { printf '  \033[31m✗\033[0m %s\n' "$*"; }
hdr()  { printf '\n\033[1m%s\033[0m\n' "$*"; }

# fcitx5 が読む data ディレクトリ群（XDG 優先順: 先頭ほど優先＝shadow する側）.
data_dirs() {
    local dirs=()
    dirs+=("${XDG_DATA_HOME:-$HOME/.local/share}")
    IFS=':' read -r -a sys <<<"${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
    dirs+=("${sys[@]}")
    printf '%s\n' "${dirs[@]}"
}

# 指定ファイル(addon|inputmethod)を全 data dir から探し、Configurable を表示.
scan() {
    local subdir="$1" fname="$2" found=0 first=1
    while IFS= read -r d; do
        local f="$d/fcitx5/$subdir/$fname"
        [ -f "$f" ] || continue
        found=1
        local cfg
        cfg=$(grep -iE '^Configurable=' "$f" | head -1 | cut -d= -f2 | tr -d '[:space:]')
        if [ "$first" = 1 ]; then
            # 最優先 (実際に使われる) コピー
            if [ "${cfg,,}" = "true" ]; then
                ok "使用される定義: $f  (Configurable=$cfg)"
            else
                bad "使用される定義: $f  (Configurable=${cfg:-未設定})  ← これが原因"
            fi
            first=0
        else
            warn "shadow されている古いコピー: $f  (Configurable=${cfg:-未設定})"
        fi
    done < <(data_dirs)
    [ "$found" = 1 ] || bad "$subdir/$fname がどの data dir にも見つからない（インストールされていない）"
}

hdr "1. fcitx5 / configtool バージョン"
fcitx5 --version 2>/dev/null | sed 's/^/  /' || warn "fcitx5 が PATH にない"

hdr "2. fcitx5 が参照する data dir（先頭ほど優先）"
data_dirs | sed 's/^/  /'

hdr "3. addon 定義 (Addons タブの歯車を決める)  fcitx5/addon/mzkana.conf"
scan addon mzkana.conf

# 注: リポジトリ上は data/mzkana-im.conf だが、CMake のインストール時に
#     inputmethod/mzkana.conf へ改名される（uniqueName = mzkana に一致させるため）。
#     よって *インストール後* に存在するファイル名は mzkana.conf。
hdr "4. input method 定義 (入力メソッドタブの歯車を決める)  fcitx5/inputmethod/mzkana.conf"
scan inputmethod mzkana.conf

hdr "5. addon 共有ライブラリと Rust FFI ライブラリ"
so_found=0
while IFS= read -r d; do
    for cand in "$d/libexec/fcitx5/addon/fcitx5-mzkana.so" \
                "$d"/fcitx5/fcitx5-mzkana.so \
                "$d"/*/fcitx5/fcitx5-mzkana.so; do
        [ -f "$cand" ] || continue
        so_found=1; ok "addon: $cand"
        if command -v ldd >/dev/null; then
            ldd "$cand" 2>/dev/null | grep -i mzkana | sed 's/^/      /'
        fi
    done
done < <(printf '%s\n' /usr /usr/local "${XDG_DATA_DIRS:-}")
[ "$so_found" = 1 ] || warn "fcitx5-mzkana.so が標準パスに見つからない（addon dir が違う可能性）"

hdr "6. 稼働中の fcitx5 が MzKana を configurable と見ているか (DBus)"
queried=0
if command -v gdbus >/dev/null 2>&1; then
    queried=1
    out=$(gdbus call --session --dest org.fcitx.Fcitx5 \
        --object-path /controller \
        --method org.fcitx.Fcitx.Controller1.GetAddons 2>/dev/null)
    if [ -n "$out" ]; then
        if grep -qi "mzkana" <<<"$out"; then
            # GetAddons の戻りは (sssibb): name, comment, ?, category, configurable, enabled
            ok "稼働中 fcitx5 は MzKana addon を認識している"
            warn "configurable/enabled の真偽は configtool 上で確認（trueなら歯車が出る）"
        else
            bad "稼働中 fcitx5 が MzKana addon を一覧に返さない → fcitx5 の再起動が必要"
        fi
    else
        warn "DBus 応答なし（fcitx5 が起動していない / セッションバス不一致）"
    fi
fi
[ "$queried" = 1 ] || warn "gdbus が無いため DBus 確認をスキップ"

hdr "判定の目安"
cat <<'EOS'
  ・3,4 で「使用される定義」が Configurable=True → 定義は正しい。
       それでも歯車が出ない場合は fcitx5 を再起動:  fcitx5 -r   （またはログインし直す）
       configtool はメタデータを起動中 fcitx5 から取得し、これは fcitx5 起動時に
       一度だけ走査されるため、再インストール後の再起動が必須。
  ・「shadow されている古いコピー」が出た → そのファイルを削除して再起動:
       例)  rm ~/.local/share/fcitx5/addon/mzkana.conf \
                ~/.local/share/fcitx5/inputmethod/mzkana.conf
  ・「使用される定義」が False/未設定 → 古い .conf を掴んでいる。再インストール:
       sudo cmake --install build   後  fcitx5 -r
  ・入力メソッドタブの歯車は「選択中の入力メソッド」がconfigurableな時のみ有効。
    Addons タブの歯車は MzKana のチェックを ON にしている時のみ押せる。
EOS
