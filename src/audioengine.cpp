// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "audioengine.h"

#include <aimic.h>

AudioEngine::AudioEngine() = default;

AudioEngine::~AudioEngine() {
    if (proc_) audio_processor_free(proc_);
    proc_ = nullptr;
}

bool AudioEngine::init(const QString &denoiseModel, const QString &tseModel,
                       const QString &aecModel, QString *errMsg) {
    if (proc_) {
        audio_processor_free(proc_);
        proc_ = nullptr;
    }
    const std::string dn = denoiseModel.toStdString();
    const std::string ts = tseModel.toStdString();
    const std::string ae = aecModel.toStdString();
    proc_ = audio_processor_new(0.0, dn.c_str(), ts.c_str(), ae.c_str());
    if (!proc_) {
        lastError_ = "音频引擎初始化失败（模型加载错误）";
        if (errMsg) *errMsg = lastError_;
        return false;
    }
    lastError_.clear();
    return true;
}

int AudioEngine::backendEffective() const {
    return proc_ ? audio_processor_backend_effective(proc_) : -1;
}

int AudioEngine::backendReason() const {
    return proc_ ? audio_processor_backend_reason(proc_) : -1;
}

void AudioEngine::setMode(int mode) {
    if (proc_) audio_processor_set_mode(proc_, mode);
}

int AudioEngine::mode() const { return proc_ ? audio_processor_get_mode(proc_) : ModeOff; }

void AudioEngine::setPreGain(double db) {
    if (proc_) audio_processor_set_pre_gain(proc_, (float)db);
}

void AudioEngine::applyEqGains(const QVector<double> &gains) {
    if (!proc_ || gains.isEmpty()) return;
    QVector<float> g(gains.size());
    for (int i = 0; i < gains.size(); ++i) g[i] = (float)gains[i];
    audio_processor_set_eq_gains(proc_, g.constData(), (size_t)g.size());
}

void AudioEngine::setCompressorEnabled(bool on) {
    if (proc_) audio_processor_set_compressor_enabled(proc_, on);
}

void AudioEngine::setAgcEnabled(bool on, double initialGainDb) {
    if (proc_) audio_processor_set_agc_enabled(proc_, on, (float)initialGainDb);
}

void AudioEngine::setVadEnabled(bool on) {
    if (proc_) audio_processor_set_vad_enabled(proc_, on);
}

void AudioEngine::setVadThreshold(double dbfs) {
    if (proc_) audio_processor_set_vad_threshold(proc_, (float)dbfs);
}

void AudioEngine::setAgcTarget(double dbfs) {
    if (proc_) audio_processor_set_agc_target(proc_, (float)dbfs);
}

void AudioEngine::setAecEnabled(bool on) {
    if (proc_) audio_processor_set_aec_enabled(proc_, on);
}

void AudioEngine::setTseEnabled(bool on) {
    if (proc_) audio_processor_set_tse_enabled(proc_, on);
}

size_t AudioEngine::process(const float *in, size_t n, const float *far, size_t farN,
                            float *out) {
    if (!proc_) return 0;
    return audio_processor_process(proc_, in, n, far, farN, out);
}
