#pragma once
#define FCITXCONFIG_EXPORT __attribute__((visibility("default")))
#define FCITXCONFIG_NO_EXPORT __attribute__((visibility("hidden")))
#define FCITXCONFIG_DEPRECATED [[deprecated]]
