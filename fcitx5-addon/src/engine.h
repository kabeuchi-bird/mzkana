#pragma once

#include <fcitx/addoninstance.h>
#include <fcitx/addonfactory.h>
#include <fcitx/inputmethodengine.h>
#include <fcitx/instance.h>
#include <fcitx/inputcontext.h>
#include <fcitx/candidatelist.h>
#include <fcitx-utils/event.h>
#include <fcitx-utils/trackableobject.h>

#include <memory>
#include <string>

extern "C" {
#include "mzkana.h"
}

namespace mzkana {

class MzkanaFcitxEngine : public fcitx::InputMethodEngineV2 {
public:
    explicit MzkanaFcitxEngine(fcitx::Instance *instance);
    ~MzkanaFcitxEngine() override;

    void keyEvent(const fcitx::InputMethodEntry &entry,
                  fcitx::KeyEvent &keyEvent) override;

    void activate(const fcitx::InputMethodEntry &entry,
                  fcitx::InputContextEvent &event) override;

    void deactivate(const fcitx::InputMethodEntry &entry,
                    fcitx::InputContextEvent &event) override;

    void reset(const fcitx::InputMethodEntry &entry,
               fcitx::InputContextEvent &event) override;

    std::string subMode(const fcitx::InputMethodEntry &entry,
                        fcitx::InputContext &ic) override;

    std::string subModeLabelImpl(const fcitx::InputMethodEntry &entry,
                                 fcitx::InputContext &ic) override;

private:
    fcitx::Instance *instance_;
    MzkanaEngine *engine_ = nullptr;
    bool mozcAvailable_ = false;

    // C2: ~10 ms periodic timer that drives mzkana_engine_tick while a preedit is
    // active (chord-confirm deadline, deferred compound-output tail). Stopped when
    // the preedit is empty to avoid a constant 100 Hz wakeup.
    std::unique_ptr<fcitx::EventSourceTime> tickTimer_;
    fcitx::TrackableObjectReference<fcitx::InputContext> tickIc_;
    std::string lastPreedit_;
    // Signature of the last-rendered candidate window (values + ids + focused
    // index). applyCandidates rebuilds the fcitx5 list only when this changes, so
    // the ~100 Hz tick doesn't reconstruct an identical window every frame.
    std::string lastCandidateSig_;

    std::string defaultConfigPath() const;
    void tryInitEngine();
    void applyResult(fcitx::InputContext *ic, const MzkanaResult &result);
    void applyCandidates(fcitx::InputContext *ic);
    bool handleCandidateKey(fcitx::InputContext *ic, const fcitx::Key &key);
    void clearPreedit(fcitx::InputContext *ic);
    void updateTickTimer(bool active, fcitx::InputContext *ic);
    void onTick();
};

class MzkanaEngineFactory : public fcitx::AddonFactory {
public:
    fcitx::AddonInstance *create(fcitx::AddonManager *manager) override;
};

} // namespace mzkana
