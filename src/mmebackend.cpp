// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "mmebackend.h"

#ifdef Q_OS_WIN

#include <windows.h>
#include <mmsystem.h>
#include <mmreg.h>

#include <algorithm>
#include <cstring>
#include <mutex>
#include <vector>

// MME 采集状态（waveIn，事件驱动）
struct MmeCaptureState {
    HWAVEIN hIn = nullptr;
    WAVEHDR bufs[4];
    std::vector<float> buffer;   // 已采集待读（48k mono float）
    std::mutex mutex;
    bool running = false;
};

// MME 播放状态（waveOut）
struct MmeRenderState {
    HWAVEOUT hOut = nullptr;
    std::vector<float> buffer;   // 待播放（48k mono float）
    std::mutex mutex;
    bool running = false;
};

namespace {
constexpr int kSampleRate = 48000;
constexpr int kChannels = 1;
constexpr int kFrameSize = 960;      // 20ms @48k
constexpr int kBufCount = 4;

WAVEFORMATEX makeFormat() {
    WAVEFORMATEX wf = {0};
    wf.wFormatTag = WAVE_FORMAT_IEEE_FLOAT;
    wf.nChannels = kChannels;
    wf.nSamplesPerSec = kSampleRate;
    wf.wBitsPerSample = 32;
    wf.nBlockAlign = kChannels * 4;
    wf.nAvgBytesPerSec = kSampleRate * kChannels * 4;
    return wf;
}

// 按设备名（friendly name）解析 MME 设备 ID
UINT findWaveInId(const QString &name) {
    if (name.isEmpty()) return WAVE_MAPPER;
    UINT n = waveInGetNumDevs();
    for (UINT i = 0; i < n; ++i) {
        WAVEINCAPS caps;
        if (waveInGetDevCaps(i, &caps, sizeof(caps)) != MMSYSERR_NOERROR) continue;
        QString cn = QString::fromWCharArray(caps.szPname);
        if (cn.contains(name)) return i;
    }
    return WAVE_MAPPER;
}

UINT findWaveOutId(const QString &name) {
    if (name.isEmpty()) return WAVE_MAPPER;
    UINT n = waveOutGetNumDevs();
    for (UINT i = 0; i < n; ++i) {
        WAVEOUTCAPS caps;
        if (waveOutGetDevCaps(i, &caps, sizeof(caps)) != MMSYSERR_NOERROR) continue;
        QString cn = QString::fromWCharArray(caps.szPname);
        if (cn.contains(name)) return i;
    }
    return WAVE_MAPPER;
}

bool openCapture(MmeCaptureState *st, const QString &name) {
    UINT dev = findWaveInId(name);
    WAVEFORMATEX wf = makeFormat();
    MMRESULT mr = waveInOpen(&st->hIn, dev, &wf, 0, 0, CALLBACK_NULL);
    if (mr != MMSYSERR_NOERROR) return false;
    for (int i = 0; i < kBufCount; ++i) {
        std::memset(&st->bufs[i], 0, sizeof(WAVEHDR));
        st->bufs[i].dwBufferLength = kFrameSize * 4;
        st->bufs[i].lpData = new char[kFrameSize * 4];
        if (waveInPrepareHeader(st->hIn, &st->bufs[i], sizeof(WAVEHDR)) != MMSYSERR_NOERROR)
            return false;
        if (waveInAddBuffer(st->hIn, &st->bufs[i], sizeof(WAVEHDR)) != MMSYSERR_NOERROR)
            return false;
    }
    waveInStart(st->hIn);
    st->running = true;
    return true;
}

bool openRender(MmeRenderState *st, const QString &name) {
    UINT dev = findWaveOutId(name);
    WAVEFORMATEX wf = makeFormat();
    MMRESULT mr = waveOutOpen(&st->hOut, dev, &wf, 0, 0, CALLBACK_NULL);
    if (mr != MMSYSERR_NOERROR) return false;
    st->running = true;
    return true;
}

// 轮询式采集（read 时从 waveIn 拉已填满的缓冲）
void pollCapture(MmeCaptureState *st) {
    for (int i = 0; i < kBufCount; ++i) {
        WAVEHDR &h = st->bufs[i];
        if (h.dwFlags & WHDR_DONE) {
            std::lock_guard<std::mutex> lock(st->mutex);
            const float *src = reinterpret_cast<const float *>(h.lpData);
            size_t n = h.dwBytesRecorded / sizeof(float);
            for (size_t j = 0; j < n; ++j) st->buffer.push_back(src[j]);
            if (st->buffer.size() > kSampleRate * 4)
                st->buffer.erase(st->buffer.begin(), st->buffer.begin() + kSampleRate);
            waveInAddBuffer(st->hIn, &h, sizeof(WAVEHDR));
        }
    }
}

void pollRender(MmeRenderState *st) {
    std::lock_guard<std::mutex> lock(st->mutex);
    if (st->buffer.empty()) return;
    // 简单策略：一次性写满一个帧
    WAVEHDR h = {0};
    size_t n = std::min((size_t)kFrameSize, st->buffer.size());
    h.lpData = reinterpret_cast<char *>(st->buffer.data());
    h.dwBufferLength = n * sizeof(float);
    MMRESULT mr = waveOutPrepareHeader(st->hOut, &h, sizeof(WAVEHDR));
    if (mr != MMSYSERR_NOERROR) return;
    if (waveOutWrite(st->hOut, &h, sizeof(WAVEHDR)) == MMSYSERR_NOERROR) {
        st->buffer.erase(st->buffer.begin(), st->buffer.begin() + n);
    }
    waveOutUnprepareHeader(st->hOut, &h, sizeof(WAVEHDR));
}

}  // namespace

MmeBackend::MmeBackend() = default;

MmeBackend::~MmeBackend() { close(); }

bool MmeBackend::open(const QString &input, const QString &output, const QString &monitor) {
    close();
    (void)monitor;
    if (!input.isEmpty()) {
        cap_ = new MmeCaptureState;
        if (!openCapture(cap_, input)) {
            lastError_ = QStringLiteral("打开 MME 采集失败");
            close();
            return false;
        }
    }
    if (!output.isEmpty()) {
        rend_ = new MmeRenderState;
        if (!openRender(rend_, output)) {
            lastError_ = QStringLiteral("打开 MME 播放失败");
            close();
            return false;
        }
    }
    active_ = true;
    return true;
}

void MmeBackend::close() {
    active_ = false;
    if (cap_) {
        cap_->running = false;
        if (cap_->hIn) {
            waveInStop(cap_->hIn);
            for (int i = 0; i < kBufCount; ++i) {
                waveInUnprepareHeader(cap_->hIn, &cap_->bufs[i], sizeof(WAVEHDR));
                delete[] cap_->bufs[i].lpData;
            }
            waveInClose(cap_->hIn);
        }
        delete cap_;
        cap_ = nullptr;
    }
    if (rend_) {
        rend_->running = false;
        if (rend_->hOut) waveOutClose(rend_->hOut);
        delete rend_;
        rend_ = nullptr;
    }
}

bool MmeBackend::active() const { return active_; }

QString MmeBackend::lastError() const { return lastError_; }

uint32_t MmeBackend::sampleRate() const { return kSampleRate; }

size_t MmeBackend::read(float *out, size_t n) {
    if (!cap_ || !cap_->running) return 0;
    pollCapture(cap_);
    std::lock_guard<std::mutex> lock(cap_->mutex);
    size_t r = std::min((size_t)cap_->buffer.size(), n);
    if (r) std::memcpy(out, cap_->buffer.data(), r * sizeof(float));
    cap_->buffer.erase(cap_->buffer.begin(), cap_->buffer.begin() + r);
    return r;
}

size_t MmeBackend::readFar(float *out, size_t n) {
    (void)out; (void)n;
    return 0;
}

void MmeBackend::write(const float *data, size_t n) {
    if (!rend_ || !rend_->running) return;
    {
        std::lock_guard<std::mutex> lock(rend_->mutex);
        rend_->buffer.insert(rend_->buffer.end(), data, data + n);
    }
    pollRender(rend_);
}

bool MmeBackend::setFar(const QString &sink, bool on) {
    (void)sink; (void)on;
    return false;
}

bool MmeBackend::setMonitor(const QString &name, bool on) {
    (void)name; (void)on;
    return false;
}

#endif // Q_OS_WIN
