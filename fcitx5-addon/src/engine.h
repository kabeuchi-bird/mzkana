#pragma once

#include <fcitx/addoninstance.h>
#include <fcitx/addonfactory.h>
#include <fcitx/inputmethodengine.h>
#include <fcitx/instance.h>
#include <fcitx/inputcontext.h>
#include <fcitx/candidatelist.h>
#include <fcitx-utils/event.h>
#include <fcitx-utils/trackableobject.h>
#include <fcitx-config/configuration.h>
#include <fcitx-config/option.h>
#include <fcitx-config/enum.h>
#include <fcitx-config/rawconfig.h>

#include <algorithm>
#include <cstdlib>
#include <filesystem>
#include <memory>
#include <string>
#include <vector>

extern "C" {
#include "mzkana.h"
}

namespace mzkana {

// Directory holding the user's layout TOML files. Both the configtool dropdown
// (LayoutFileAnnotation) and the engine resolve layout paths against this dir,
// so the location lives in one place.
inline std::string layoutDir() {
    const char *home = std::getenv("HOME");
    if (!home) {
        home = "/root";
    }
    return std::string(home) + "/.config/fcitx5/conf/mzkana";
}

// Enumerate `*.toml` files in the layout directory (file names only, sorted).
// Used to populate the configtool dropdown at runtime.
inline std::vector<std::string> listLayoutTomlFiles() {
    std::vector<std::string> result;
    std::error_code ec;
    std::filesystem::directory_iterator it(layoutDir(), ec);
    if (ec) {
        return result;
    }
    for (const auto &entry : it) {
        if (!entry.is_regular_file(ec)) {
            continue;
        }
        const auto &p = entry.path();
        if (p.extension() == ".toml") {
            result.push_back(p.filename().string());
        }
    }
    std::sort(result.begin(), result.end());
    return result;
}

// §13.5: tells configtool that this string option is an enumeration whose values
// are discovered at runtime (the .toml files in layoutDir()). Same idiom as
// fcitx5-rime's schema dropdown.
struct LayoutFileAnnotation : public fcitx::EnumAnnotation {
    void dumpDescription(fcitx::RawConfig &config) const {
        fcitx::EnumAnnotation::dumpDescription(config); // sets IsEnum=True
        int i = 0;
        for (const auto &name : listLayoutTomlFiles()) {
            config.setValueByPath("Enum/" + std::to_string(i), name);
            config.setValueByPath("EnumI18n/" + std::to_string(i), name);
            ++i;
        }
    }
};

// Addon configuration exposed to fcitx5-configtool. Currently only the layout
// file selection; further options (chord windows, preedit fallback, …) can be
// added here as additional Option members.
FCITX_CONFIGURATION(
    MzkanaConfig,
    fcitx::OptionWithAnnotation<std::string, LayoutFileAnnotation> layout{
        this, "Layout", "配列ファイル", "layout.toml"};);

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

    // §13.5: configtool integration. getConfig exposes the settings schema;
    // setConfig is called on Apply; reloadConfig on external .conf changes.
    const fcitx::Configuration *getConfig() const override { return &config_; }
    void setConfig(const fcitx::RawConfig &raw) override;
    void reloadConfig() override;

    void clearPreedit(fcitx::InputContext *ic);

private:
    fcitx::Instance *instance_;
    MzkanaConfig config_;
    MzkanaEngine *engine_ = nullptr;
    bool mozcAvailable_ = false;

    std::unique_ptr<fcitx::EventSourceTime> tickTimer_;
    fcitx::TrackableObjectReference<fcitx::InputContext> tickIc_;
    std::string lastPreedit_;
    std::string lastCandidateSig_;

    // Full path of the layout file currently selected in config_.
    std::string currentLayoutPath() const;
    // Apply config_.layout to the running engine (create or hot-swap the layout).
    void reloadSelectedLayout();
    void tryInitEngine();
    void applyResult(fcitx::InputContext *ic, const MzkanaResult &result);
    void applyCandidates(fcitx::InputContext *ic);
    bool handleCandidateKey(fcitx::InputContext *ic, const fcitx::Key &key);
    void updateTickTimer(bool active, fcitx::InputContext *ic);
    void onTick();
};

class MzkanaEngineFactory : public fcitx::AddonFactory {
public:
    fcitx::AddonInstance *create(fcitx::AddonManager *manager) override;
};

} // namespace mzkana
