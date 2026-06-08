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

// Configtool enums. The first value ("FollowLayout") maps to the -1 sentinel
// on the FFI side, meaning "no override — use the TOML [settings] value".
enum class PreeditFallbackOption { FollowLayout, Client, Panel, Buffer, Auto };
FCITX_CONFIG_ENUM_NAME(PreeditFallbackOption,
    "配列設定に従う", "クライアント", "パネル", "バッファ", "自動")

enum class OnFocusChangeOption { FollowLayout, Preserve, Reset };
FCITX_CONFIG_ENUM_NAME(OnFocusChangeOption,
    "配列設定に従う", "保持する", "リセットする")

enum class ShowPredictionOption { FollowLayout, Show, Hide };
FCITX_CONFIG_ENUM_NAME(ShowPredictionOption,
    "配列設定に従う", "表示する", "表示しない")

FCITX_CONFIGURATION(
    MzkanaConfig,
    fcitx::OptionWithAnnotation<std::string, LayoutFileAnnotation> layout{
        this, "Layout", "配列ファイル", "layout.toml"};
    fcitx::Option<PreeditFallbackOption> preeditFallback{
        this, "PreeditFallback", "Preedit表示方式",
        PreeditFallbackOption::FollowLayout};
    fcitx::Option<OnFocusChangeOption> onFocusChange{
        this, "OnFocusChange", "フォーカス変更時の動作",
        OnFocusChangeOption::FollowLayout};
    fcitx::Option<int> candidatePageSize{
        this, "CandidatePageSize",
        "候補ページサイズ (0=配列設定に従う)", 0};
    fcitx::Option<ShowPredictionOption> showPrediction{
        this, "ShowPrediction", "予測候補の表示",
        ShowPredictionOption::FollowLayout};);

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
    // Addon tab uses getConfig/setConfig; Input Method page uses the
    // *ForInputMethod variants (default returns nullptr → no gear icon).
    const fcitx::Configuration *getConfig() const override { return &config_; }
    void setConfig(const fcitx::RawConfig &raw) override;
    void reloadConfig() override;

    const fcitx::Configuration *
    getConfigForInputMethod(const fcitx::InputMethodEntry &) const override {
        return &config_;
    }
    void setConfigForInputMethod(const fcitx::InputMethodEntry &,
                                 const fcitx::RawConfig &raw) override {
        setConfig(raw);
    }

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
    // Push configtool override values to the engine via FFI setters.
    void applyConfigOverrides();
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
