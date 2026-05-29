#include "engine.h"

#include <fcitx-utils/key.h>
#include <fcitx-utils/keysym.h>
#include <fcitx/addonmanager.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputmethodentry.h>
#include <fcitx/inputpanel.h>
#include <fcitx/text.h>

#include <cstdlib>

namespace mzkana {

// ── MzkanaFcitxEngine ─────────────────────────────────────────────────────────

MzkanaFcitxEngine::MzkanaFcitxEngine(fcitx::Instance *instance)
    : instance_(instance) {
    tryInitEngine();
}

void MzkanaFcitxEngine::tryInitEngine() {
    if (engine_) return;
    engine_ = mzkana_engine_create(defaultConfigPath().c_str(), nullptr);
    // engine_ may still be null if the layout file doesn't exist yet; retry
    // on the next keyEvent or activate call.
    mozcAvailable_ = engine_ && mzkana_engine_mozc_available(engine_);
}

MzkanaFcitxEngine::~MzkanaFcitxEngine() {
    mzkana_engine_destroy(engine_);
}

std::string MzkanaFcitxEngine::defaultConfigPath() const {
    const char *home = std::getenv("HOME");
    if (!home) home = "/root";
    return std::string(home) + "/.config/fcitx5/conf/mzkana/layout.toml";
}

void MzkanaFcitxEngine::keyEvent(const fcitx::InputMethodEntry & /*entry*/,
                                  fcitx::KeyEvent &keyEvent) {
    tryInitEngine();
    if (!engine_) return;

    auto *ic = keyEvent.inputContext();
    const fcitx::Key &key = keyEvent.key();

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

    // Hot-reload: check on every key event (inotify debounce makes this cheap)
    mzkana_engine_check_reload(engine_);

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

void MzkanaFcitxEngine::deactivate(const fcitx::InputMethodEntry &entry,
                                    fcitx::InputContextEvent &event) {
    reset(entry, event);
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

    // Only re-render the preedit when it actually changed. This keeps the ~100 Hz
    // tick timer (C2) cheap: a pure pruning/deadline tick that leaves the preedit
    // unchanged costs no UI update.
    if (preedit != lastPreedit_) {
        fcitx::Text preeditText;
        if (!preedit.empty()) {
            preeditText.append(preedit, fcitx::TextFormatFlag::Underline);
            preeditText.setCursor(static_cast<int>(preedit.size()));
        }
        // setClientPreedit → inline in capable clients
        // setPreedit → floating panel as fallback
        ic->inputPanel().setClientPreedit(preeditText);
        ic->inputPanel().setPreedit(preeditText);
        ic->updatePreedit();
        ic->updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
        lastPreedit_ = preedit;
    }

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

void MzkanaFcitxEngine::clearPreedit(fcitx::InputContext *ic) {
    ic->inputPanel().setClientPreedit(fcitx::Text());
    ic->inputPanel().setPreedit(fcitx::Text());
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
