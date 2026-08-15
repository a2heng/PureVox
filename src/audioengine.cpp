// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "audioengine.h"

AudioEngine::AudioEngine() = default;

AudioEngine::~AudioEngine() = default;

bool AudioEngine::init(const QString &denoiseModel, const QString &tseModel,
                       const QString &aecModel, QString *errMsg) {
    QMutexLocker lock(&mutex_);
    proc_ = std::make_unique<pv::Processor>();
    proc_->init(denoiseModel.toStdString(), tseModel.toStdString(), aecModel.toStdString());
    if (!proc_->tseAvailable() && !proc_->aecAvailable() && proc_->mode() != pv::Processor::ModeDenoise) {
        lastError_ = "音频引擎初始化失败（模型加载错误）";
        if (errMsg) *errMsg = lastError_;
        return false;
    }
    lastError_.clear();
    return true;
}

int AudioEngine::backendEffective() const {
    return proc_ ? proc_->backendEffective() : -1;
}

int AudioEngine::backendReason() const {
    return proc_ ? proc_->backendReason() : -1;
}

void AudioEngine::cleanup() {
    QMutexLocker lock(&mutex_);
    proc_.reset();
}

void AudioEngine::replayParams(int mode, double preGain, double postGain,
                               const QVector<double> &eqGains, bool compOn, bool agcOn,
                               bool vadOn, bool monitorOn) {
    if (!proc_) return;
    proc_->setMode(mode);
    proc_->setPreGain((float)preGain);
    proc_->setPostGain((float)postGain);
    if (!eqGains.isEmpty()) {
        QVector<float> g(eqGains.size());
        for (int i = 0; i < eqGains.size(); ++i) g[i] = (float)eqGains[i];
        proc_->setEqGains(g.constData(), (size_t)g.size());
    }
    proc_->setCompressorEnabled(compOn);
    proc_->setAgcEnabled(agcOn, (float)preGain);
    proc_->setVadEnabled(vadOn);
    (void)monitorOn;
}

void AudioEngine::setMode(int mode) {
    if (proc_) proc_->setMode(mode);
}

int AudioEngine::mode() const { return proc_ ? proc_->mode() : pv::Processor::ModeOff; }

void AudioEngine::setPreGain(double db) {
    if (proc_) proc_->setPreGain((float)db);
}

void AudioEngine::setPostGain(double db) {
    if (proc_) proc_->setPostGain((float)db);
}

void AudioEngine::applyEqGains(const QVector<double> &gains) {
    if (!proc_ || gains.isEmpty()) return;
    QVector<float> g(gains.size());
    for (int i = 0; i < gains.size(); ++i) g[i] = (float)gains[i];
    proc_->setEqGains(g.constData(), (size_t)g.size());
}

void AudioEngine::setCompressorEnabled(bool on) {
    if (proc_) proc_->setCompressorEnabled(on);
}

void AudioEngine::setAgcEnabled(bool on, double initialGainDb) {
    if (proc_) proc_->setAgcEnabled(on, (float)initialGainDb);
}

void AudioEngine::setVadEnabled(bool on) {
    if (proc_) proc_->setVadEnabled(on);
}

void AudioEngine::setVadThreshold(double dbfs) {
    if (proc_) proc_->setVadThreshold((float)dbfs);
}

void AudioEngine::setAgcTarget(double dbfs) {
    if (proc_) proc_->setAgcTarget((float)dbfs);
}

void AudioEngine::setAecEnabled(bool on) {
    if (proc_) proc_->setAecEnabled(on);
}

void AudioEngine::setTseEnabled(bool on) {
    if (proc_) proc_->setTseEnabled(on);
}

void AudioEngine::setTseReference(const float *data, size_t n) {
    QMutexLocker lock(&mutex_);
    if (proc_) proc_->setTseReference(data, n);
}

bool AudioEngine::tseReferenceLoaded() const {
    QMutexLocker lock(&mutex_);
    return proc_ && proc_->tseReferenceLoaded();
}

size_t AudioEngine::process(const float *in, size_t n, const float *far, size_t farN,
                            float *out) {
    QMutexLocker lock(&mutex_);
    if (!proc_) return 0;
    return proc_->process(in, n, far, farN, out);
}

size_t AudioEngine::vizInputTake(float *out, size_t cap) {
    QMutexLocker lock(&mutex_);
    return proc_ ? proc_->vizInputTake(out, cap) : 0;
}

size_t AudioEngine::vizOutputTake(float *out, size_t cap) {
    QMutexLocker lock(&mutex_);
    return proc_ ? proc_->vizOutputTake(out, cap) : 0;
}
