#include "engine.h"

#include <fcitx-utils/key.h>
#include <fcitx-utils/keysym.h>
#include <fcitx-utils/capabilityflags.h>
#include <fcitx-utils/log.h>
#include <fcitx-config/iniparser.h>
#include <fcitx/addonmanager.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputmethodentry.h>
#include <fcitx/inputpanel.h>
#include <fcitx/text.h>

#include <cstdlib>
#include <string>
#include <vector>

namespace mzkana {

// ── MzkanaFcitxEngine ─────────────────────────────────────────────────────────

MzkanaFcitxEngine::MzkanaFcitxEngine(fcitx::Instance *instance)
    : instance_(instance) {
    // Load persisted settings (conf/mzkana.conf), falling back to defaults, then
    // create the engine for the selected layout.
    reloadConfig();
}

void MzkanaFcitxEngine::tryInitEngine() {
    if (engine_) return;
    engine_ = mzkana_engine_create(currentLayoutPath().c_str(), nullptr);
    // engine_ may still be null if the layout file doesn't exist yet; retry
    // on the next keyEvent or activate call.
    mozcAvailable_ = engine_ && mzkana_engine_mozc_available(engine_);
}

MzkanaFcitxEngine::~MzkanaFcitxEngine() {
    mzkana_engine_destroy(engine_);
}

std::string MzkanaFcitxEngine::currentLayoutPath() const {
    return layoutDir() + "/" + config_.layout.value();
}

// §13.5: apply the configtool selection. Hot-swaps the layout in the running
// engine (Mozc connection preserved); creates the engine if it didn't exist yet
// (e.g. the file was missing at startup but has since appeared).
bool MzkanaFcitxEngine::reloadSelectedLayout() {
    const std::string path = currentLayoutPath();
    if (engine_) {
        if (path != lastLayoutPath_) {
            if (!mzkana_engine_reload_layout(engine_, path.c_str())) {
                FCITX_WARN() << "mzkana: failed to load layout: " << path;
                return false;
            }
            lastLayoutPath_ = path;
        }
    } else {
        engine_ = mzkana_engine_create(path.c_str(), nullptr);
        if (!engine_) {
            return false;
        }
        lastLayoutPath_ = path;
    }
    mozcAvailable_ = engine_ && mzkana_engine_mozc_available(engine_);
    applyConfigOverrides();
    return true;
}

void MzkanaFcitxEngine::applyConfigOverrides() {
    if (!engine_) return;

    // preedit_fallback: FollowLayout → -1 (use TOML), Client → 0, …
    mzkana_engine_set_preedit_fallback(
        engine_,
        static_cast<int>(*config_.preeditFallback) - 1);

    // on_focus_change: FollowLayout → -1, Preserve → 0, Reset → 1
    mzkana_engine_set_on_focus_change(
        engine_,
        static_cast<int>(*config_.onFocusChange) - 1);

    // candidate_page_size: 0 = use TOML default
    mzkana_engine_set_candidate_page_size(
        engine_,
        *config_.candidatePageSize);

    // show_prediction: FollowLayout → -1, Show → 1, Hide → 0
    int showPred;
    switch (*config_.showPrediction) {
    case ShowPredictionOption::Show:  showPred = 1;  break;
    case ShowPredictionOption::Hide:  showPred = 0;  break;
    default:                          showPred = -1; break;
    }
    mzkana_engine_set_show_prediction(engine_, showPred);
}

void MzkanaFcitxEngine::setConfig(const fcitx::RawConfig &raw) {
    MzkanaConfig prev = config_;
    config_.load(raw, true);
    if (!reloadSelectedLayout()) {
        config_ = prev;
        return;
    }
    fcitx::safeSaveAsIni(config_, "conf/mzkana.conf");
}

void MzkanaFcitxEngine::reloadConfig() {
    fcitx::readAsIni(config_, "conf/mzkana.conf");
    reloadSelectedLayout();
}

void MzkanaFcitxEngine::keyEvent(const fcitx::InputMethodEntry & /*entry*/,
                                  fcitx::KeyEvent &keyEvent) {
    tryInitEngine();
    if (!engine_) return;

    auto *ic = keyEvent.inputContext();
    const fcitx::Key &key = keyEvent.key();

    // H2: sensitive (password) fields. Honor sensitive_field_behavior:
    //   passthrough(0) → don't run the IME, let the app handle keys (nothing exposed).
    //   buffer(1)      → process normally but the preedit is never shown in such
    //                    fields (applyResult suppresses it below), so kana still
    //                    composes without leaking to the panel.
    if (ic->capabilityFlags().test(fcitx::CapabilityFlag::Password)) {
        if (mzkana_engine_sensitive_field_behavior(engine_) == 0) {
            return;
        }
    }

    // Skip Ctrl / Alt / Super / Hyper — let the application handle these.
    fcitx::KeyStates badMods = {
        fcitx::KeyState::Ctrl,
        fcitx::KeyState::Alt,
        fcitx::KeyState::Super,
        fcitx::KeyState::Hyper,
    };
    if (key.states() & badMods) {
        return;
    }

    // Skip bare modifier key events (Shift_L, Ctrl_L, etc.)
    if (key.isModifier()) return;

    // Candidate window navigation (Phase 4): when a candidate list is shown, a
    // number key directly selects a candidate. Space/arrows/Enter fall through to
    // normal processing — they are forwarded to Mozc, which moves its focus and
    // returns an updated candidate window that applyCandidates() re-renders.
    if (!keyEvent.isRelease() && handleCandidateKey(ic, key)) {
        keyEvent.filterAndAccept();
        return;
    }

    bool shift = bool(key.states() & fcitx::KeyState::Shift);

    // Normalize keysym to level-0: uppercase letters → lowercase.
    fcitx::KeySym sym = key.sym();
    if (sym >= FcitxKey_A && sym <= FcitxKey_Z) {
        sym = static_cast<fcitx::KeySym>(
            static_cast<unsigned>(sym) +
            (static_cast<unsigned>(FcitxKey_a) -
             static_cast<unsigned>(FcitxKey_A)));
    }
    const std::string keyName = fcitx::Key::keySymToString(sym);
    if (keyName.empty()) return;

    MzkanaResult result;
    if (keyEvent.isRelease()) {
        result = mzkana_engine_key_up(engine_, keyName.c_str());
    } else {
        result = mzkana_engine_key_down(engine_, keyName.c_str(), shift ? 1 : 0);
    }

    applyResult(ic, result);

    if (result.consumed) {
        keyEvent.filterAndAccept();
    }

    // Hot-reload: check on every key event (inotify debounce makes this cheap).
    // On reload the composition is reverted in the engine, so clear the now-stale
    // preedit from the UI too (H4).
    if (mzkana_engine_check_reload(engine_)) {
        clearPreedit(ic);
    }

    // Update status area if Mozc availability changed (e.g. reconnected after startup).
    bool nowAvailable = mzkana_engine_mozc_available(engine_);
    if (nowAvailable != mozcAvailable_) {
        mozcAvailable_ = nowAvailable;
        ic->updateUserInterface(fcitx::UserInterfaceComponent::StatusArea);
    }
}

void MzkanaFcitxEngine::activate(const fcitx::InputMethodEntry & /*entry*/,
                                   fcitx::InputContextEvent &event) {
    tryInitEngine();
    // Refresh Mozc availability (may have changed since last activation).
    bool nowAvailable = engine_ && mzkana_engine_mozc_available(engine_);
    if (nowAvailable != mozcAvailable_) {
        mozcAvailable_ = nowAvailable;
    }
    event.inputContext()->updateUserInterface(
        fcitx::UserInterfaceComponent::StatusArea);
}

void MzkanaFcitxEngine::deactivate(const fcitx::InputMethodEntry & /*entry*/,
                                    fcitx::InputContextEvent &event) {
    bool preserved = false;
    if (engine_) {
        preserved = mzkana_engine_focus_out(engine_) != 0;
    }
    if (!preserved) {
        clearPreedit(event.inputContext());
    }
}

void MzkanaFcitxEngine::reset(const fcitx::InputMethodEntry & /*entry*/,
                               fcitx::InputContextEvent &event) {
    if (engine_) {
        mzkana_engine_reset(engine_);
    }
    clearPreedit(event.inputContext());
}

void MzkanaFcitxEngine::applyResult(fcitx::InputContext *ic,
                                     const MzkanaResult &result) {
    // passthrough_key: side-effect key forwarded after consuming the original
    // event (e.g. BackSpace from chord rewrite to undo speculatively-committed
    // kana). Must be applied BEFORE commitString so the BackSpace deletes the
    // old text before the new commit appends to it.
    //
    // Only forward when consumed=true; if consumed=false, fcitx5 lets the
    // natural key event reach the app already, and forwarding would double it.
    if (result.consumed && result.passthrough_key_len > 0) {
        std::string keyName(
            reinterpret_cast<const char *>(result.passthrough_key),
            result.passthrough_key_len);
        auto sym = fcitx::Key::keySymFromString(keyName);
        if (sym != FcitxKey_None) {
            ic->forwardKey(fcitx::Key(sym));
        }
    }

    if (result.commit_len > 0) {
        ic->commitString(std::string(
            reinterpret_cast<const char *>(result.commit), result.commit_len));
    }

    std::string preedit(
        reinterpret_cast<const char *>(result.preedit), result.preedit_len);

    // Build a compact signature of preedit content + segment annotations so
    // re-rendering is skipped when nothing visually changed (e.g. tick timer).
    // Defense in depth: the FFI contract caps segment_count at
    // MAX_PREEDIT_SEGMENTS, but clamp here so a bad count can never index past
    // the fixed-size preedit_segments array.
    const uint32_t segCount =
        std::min<uint32_t>(result.preedit_segment_count, MAX_PREEDIT_SEGMENTS);
    std::string preeditSig = preedit;
    preeditSig += '\x1e';
    for (uint32_t i = 0; i < segCount; ++i) {
        preeditSig += static_cast<char>(result.preedit_segments[i].attr);
        preeditSig += static_cast<char>(result.preedit_segments[i].len & 0xff);
    }

    if (preeditSig != lastPreedit_) {
        // H2: never show a preedit in a password/sensitive field — it would leak
        // what is being typed to the floating panel.
        const bool sensitive =
            ic->capabilityFlags().test(fcitx::CapabilityFlag::Password);

        fcitx::Text preeditText;
        if (!preedit.empty() && !sensitive) {
            if (segCount > 0) {
                for (uint32_t i = 0; i < segCount; ++i) {
                    const auto &seg = result.preedit_segments[i];
                    // Defense in depth: never read past the copied preedit bytes,
                    // even if the engine ever hands back an inconsistent offset.
                    if (seg.start > result.preedit_len ||
                        seg.len > result.preedit_len - seg.start) {
                        continue;
                    }
                    std::string segStr(
                        reinterpret_cast<const char *>(result.preedit) + seg.start,
                        seg.len);
                    fcitx::TextFormatFlags flags;
                    if (seg.attr == 2) {
                        flags = fcitx::TextFormatFlag::HighLight;
                    } else {
                        flags = fcitx::TextFormatFlag::Underline;
                    }
                    preeditText.append(segStr, flags);
                }
            } else {
                preeditText.append(preedit, fcitx::TextFormatFlag::Underline);
            }
            preeditText.setCursor(static_cast<int>(preedit.size()));
        }

        // Preedit display:
        //   - Clients that support inline preedit always get it inline
        //     (setClientPreedit), which is what most apps render in-place.
        //   - For clients that cannot, preedit_fallback decides the fallback:
        //     buffer(2) shows nothing; client/panel/auto show the floating panel.
        const bool clientCapable =
            ic->capabilityFlags().test(fcitx::CapabilityFlag::Preedit);

        if (clientCapable) {
            // Inline preedit in the client. Also mirror to the panel so it stays
            // visible if the client chooses not to render it.
            ic->inputPanel().setClientPreedit(preeditText);
            ic->inputPanel().setPreedit(preeditText);
        } else if (mzkana_engine_preedit_fallback(engine_) == 2 /* buffer */) {
            // Keep composing internally but show nothing.
            ic->inputPanel().setClientPreedit(fcitx::Text());
            ic->inputPanel().setPreedit(fcitx::Text());
        } else {
            // Floating panel fallback (client/panel/auto for non-inline clients).
            ic->inputPanel().setClientPreedit(fcitx::Text());
            ic->inputPanel().setPreedit(preeditText);
        }
        ic->updatePreedit();
        ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
        lastPreedit_ = std::move(preeditSig);
    }

    // Reflect the latest Mozc candidate window (prediction or conversion).
    applyCandidates(ic);

    // Run the tick timer while a preedit is active so chord-confirm deadlines and
    // deferred compound-output tails fire even without further key input.
    updateTickTimer(!preedit.empty(), ic);

    if (result.forward_key_len > 0) {
        std::string keyName(reinterpret_cast<const char *>(result.forward_key),
                            result.forward_key_len);
        fcitx::KeyStates states;
        if (result.forward_modifiers & 0x01) states |= fcitx::KeyState::Shift;
        if (result.forward_modifiers & 0x02) states |= fcitx::KeyState::Ctrl;
        if (result.forward_modifiers & 0x04) states |= fcitx::KeyState::Alt;
        if (result.forward_modifiers & 0x08) states |= fcitx::KeyState::Super;
        auto sym = fcitx::Key::keySymFromString(keyName);
        if (sym != FcitxKey_None) {
            ic->forwardKey(fcitx::Key(sym, states));
        }
    }
}

std::string MzkanaFcitxEngine::subMode(const fcitx::InputMethodEntry &,
                                        fcitx::InputContext &) {
    return mozcAvailable_ ? "（Mozc）" : "（変換エンジン未起動）";
}

std::string MzkanaFcitxEngine::subModeLabelImpl(const fcitx::InputMethodEntry &,
                                                  fcitx::InputContext &) {
    return mozcAvailable_ ? "MzKana（Mozc）" : "MzKana（変換エンジン未起動）";
}

// ── Candidate window (Phase 4) ──────────────────────────────────────────────────

namespace {

// A candidate backed by a Mozc-internal id. Selecting it asks the engine to
// commit that id; the engine returns the committed text which the engine class
// applies. The select() callback stores the id and engine pointer so it can call
// back through the C ABI.
class MzkanaCandidateWord : public fcitx::CandidateWord {
public:
    MzkanaCandidateWord(MzkanaEngine *engine, MzkanaFcitxEngine *host,
                        int32_t id, const std::string &text,
                        const std::string &comment)
        : engine_(engine), host_(host), id_(id) {
        fcitx::Text t(text);
        setText(std::move(t));
        if (!comment.empty()) {
            setComment(fcitx::Text(comment));
        }
    }

    void select(fcitx::InputContext *ic) const override {
        if (engine_ == nullptr) {
            return;
        }
        if (id_ < 0) {
            return;
        }
        MzkanaResult r = mzkana_engine_select_candidate(engine_, id_);
        if (r.commit_len > 0) {
            ic->commitString(std::string(
                reinterpret_cast<const char *>(r.commit), r.commit_len));
        }
        // Selection ends composition: fully clean up C++ state so the tick
        // timer stops and preedit/candidate caches are reset.
        if (host_) {
            host_->clearPreedit(ic);
        } else {
            ic->inputPanel().reset();
            ic->updatePreedit();
            ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
        }
    }

private:
    MzkanaEngine *engine_;
    MzkanaFcitxEngine *host_;
    int32_t id_;
};

} // namespace

void MzkanaFcitxEngine::applyCandidates(fcitx::InputContext *ic) {
    uint32_t n = mzkana_engine_candidate_count(engine_);
    int32_t focused = mzkana_engine_focused_index(engine_);

    // Suppress the prediction/suggestion window (focused == -1) when
    // show_prediction is off. Conversion candidates (focused >= 0) are
    // always shown — hiding them would make conversion unusable.
    if (n > 0 && focused < 0 && !mzkana_engine_show_prediction(engine_)) {
        n = 0;
    }

    if (n == 0) {
        // No window: drop any existing candidate list (only touch the UI if there
        // is one to clear, so empty ticks stay cheap).
        if (!lastCandidateSig_.empty() || ic->inputPanel().candidateList()) {
            ic->inputPanel().setCandidateList(nullptr);
            ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
            lastCandidateSig_.clear();
        }
        return;
    }

    // Build a signature of value/id per candidate plus the focused index. If it
    // matches the last render, the window is unchanged — skip the rebuild so the
    // ~100 Hz tick doesn't thrash the candidate list and panel.
    std::string sig;
    sig.reserve(n * 8);
    sig += std::to_string(focused);
    sig += '|';
    struct Entry { std::string value, comment; int32_t id; };
    std::vector<Entry> entries;
    entries.reserve(n);
    for (uint32_t i = 0; i < n; ++i) {
        MzkanaCandidate c = mzkana_engine_candidate(engine_, i);
        std::string value(reinterpret_cast<const char *>(c.value), c.value_len);
        std::string comment;
        if (c.annotation && c.annotation_len > 0) {
            comment.assign(reinterpret_cast<const char *>(c.annotation),
                           c.annotation_len);
        }
        sig += std::to_string(c.id);
        sig += ':';
        sig += value;
        sig += '\x1e';
        sig += comment; // include annotation so changes to it trigger a rebuild
        sig += '\x1f';
        entries.push_back({std::move(value), std::move(comment), c.id});
    }
    if (sig == lastCandidateSig_) {
        return; // unchanged — nothing to do
    }
    lastCandidateSig_ = std::move(sig);

    auto list = std::make_unique<fcitx::CommonCandidateList>();
    int pageSize = mzkana_engine_candidate_page_size(engine_);
    if (pageSize < 1 || pageSize > 9) pageSize = 5;
    list->setPageSize(pageSize);
    static const auto kSelectionKeys =
        fcitx::Key::keyListFromString("1 2 3 4 5 6 7 8 9");
    list->setSelectionKey(kSelectionKeys);
    // Mozc's conversion/prediction window is vertical.
    list->setLayoutHint(fcitx::CandidateLayoutHint::Vertical);

    for (auto &e : entries) {
        list->append<MzkanaCandidateWord>(engine_, this, e.id, e.value, e.comment);
    }

    // During conversion Mozc reports the focused index; highlight it.
    if (focused >= 0 && static_cast<uint32_t>(focused) < n) {
        list->setGlobalCursorIndex(focused);
    }

    ic->inputPanel().setCandidateList(std::move(list));
    ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
}

bool MzkanaFcitxEngine::handleCandidateKey(fcitx::InputContext *ic,
                                            const fcitx::Key &key) {
    auto candidateList = ic->inputPanel().candidateList();
    if (!candidateList) return false;

    // Number keys 1..9 select the candidate at that position on the current page.
    static const auto kSelectionKeys =
        fcitx::Key::keyListFromString("1 2 3 4 5 6 7 8 9");
    int idx = key.keyListIndex(kSelectionKeys);
    if (idx >= 0 && idx < candidateList->size()) {
        candidateList->candidate(idx).select(ic);
        return true;
    }
    // Space / arrows / Enter fall through to normal processing so they reach Mozc,
    // which advances its focus and returns an updated window (re-rendered then).
    return false;
}

void MzkanaFcitxEngine::clearPreedit(fcitx::InputContext *ic) {
    ic->inputPanel().setClientPreedit(fcitx::Text());
    ic->inputPanel().setPreedit(fcitx::Text());
    // Also drop any candidate window and its render cache, otherwise a stale list
    // lingers after reset/deactivate and onTick keeps rebuilding it at ~100 Hz.
    ic->inputPanel().setCandidateList(nullptr);
    lastCandidateSig_.clear();
    ic->updatePreedit();
    ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
    lastPreedit_.clear();
    // No preedit left → stop the tick timer.
    if (tickTimer_) {
        tickTimer_->setEnabled(false);
    }
}

// ── C2: periodic tick wiring ────────────────────────────────────────────────────

void MzkanaFcitxEngine::updateTickTimer(bool active, fcitx::InputContext *ic) {
    constexpr uint64_t kTickIntervalUs = 10000; // 10 ms

    if (!active) {
        if (tickTimer_) {
            tickTimer_->setEnabled(false);
        }
        return;
    }

    // Remember which input context the timer callback should drive.
    tickIc_ = ic->watch();

    if (!tickTimer_) {
        tickTimer_ = instance_->eventLoop().addTimeEvent(
            CLOCK_MONOTONIC, fcitx::now(CLOCK_MONOTONIC) + kTickIntervalUs, 0,
            [this](fcitx::EventSourceTime *, uint64_t) {
                // onTick() runs applyResult(), which re-arms or stops this timer
                // via updateTickTimer() depending on whether a preedit remains.
                onTick();
                return true;
            });
    } else {
        tickTimer_->setNextInterval(kTickIntervalUs);
        tickTimer_->setOneShot();
        tickTimer_->setEnabled(true);
    }
}

void MzkanaFcitxEngine::onTick() {
    if (!engine_) {
        return;
    }
    auto *ic = tickIc_.get();
    if (!ic) {
        // Input context went away — stop ticking.
        if (tickTimer_) {
            tickTimer_->setEnabled(false);
        }
        return;
    }
    applyResult(ic, mzkana_engine_tick(engine_));
}

// ── Factory ───────────────────────────────────────────────────────────────────

fcitx::AddonInstance *
MzkanaEngineFactory::create(fcitx::AddonManager *manager) {
    return new MzkanaFcitxEngine(manager->instance());
}

} // namespace mzkana

FCITX_ADDON_FACTORY(mzkana::MzkanaEngineFactory)
