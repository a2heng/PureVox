// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "wasapibackend.h"

#ifdef Q_OS_WIN

#include <windows.h>
#include <mmdeviceapi.h>
#include <audioclient.h>
#include <audiopolicy.h>
#include <functiondiscoverykeys_devpkey.h>
#include <avrt.h>

#include <algorithm>
#include <cstring>
#include <mutex>
#include <vector>

// WASAPI 采集状态
struct WasapiCaptureState {
    IMMDevice *device = nullptr;
    IAudioClient *client = nullptr;
    IAudioCaptureClient *capture = nullptr;
    std::mutex mutex;
    std::vector<float> buffer;   // 已采集待读
    UINT32 frameSize = 0;
    bool running = false;
    bool mono = false;
    DWORD outCh = 1;
};

// WASAPI 播放状态
struct WasapiRenderState {
    IMMDevice *device = nullptr;
    IAudioClient *client = nullptr;
    IAudioRenderClient *render = nullptr;
    std::mutex mutex;
    std::vector<float> buffer;   // 待播放
    UINT32 frameSize = 0;
    bool running = false;
};

namespace {
constexpr int kSampleRate = 48000;
constexpr int kChannels = 1;

IMMDevice *findDevice(bool capture) {
    CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    IMMDeviceEnumerator *en = nullptr;
    if (CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL,
                         __uuidof(IMMDeviceEnumerator), (void **)&en) != S_OK)
        return nullptr;
    IMMDevice *dev = nullptr;
    en->GetDefaultAudioEndpoint(capture ? eCapture : eRender, eConsole, &dev);
    en->Release();
    return dev;
}

// 设备名 → 设备（按 friendly name 匹配）
IMMDevice *findDeviceByName(const QString &name, bool capture) {
    if (name.isEmpty()) return findDevice(capture);
    CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    IMMDeviceEnumerator *en = nullptr;
    if (CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL,
                         __uuidof(IMMDeviceEnumerator), (void **)&en) != S_OK)
        return nullptr;
    IMMDeviceCollection *coll = nullptr;
    IMMDevice *found = nullptr;
    if (en->EnumAudioEndpoints(capture ? eCapture : eRender, DEVICE_STATE_ACTIVE, &coll) == S_OK) {
        UINT count = 0;
        coll->GetCount(&count);
        for (UINT i = 0; i < count; ++i) {
            IMMDevice *d = nullptr;
            if (coll->Item(i, &d) != S_OK) continue;
            IPropertyStore *ps = nullptr;
            if (d->OpenPropertyStore(STGM_READ, &ps) == S_OK) {
                PROPVARIANT pv;
                PropVariantInit(&pv);
                if (ps->GetValue(PKEY_Device_FriendlyName, &pv) == S_OK &&
                    pv.vt == VT_LPWSTR && pv.pwszVal) {
                    QString n = QString::fromWCharArray(pv.pwszVal);
                    if (n.contains(name)) { found = d; }
                    PropVariantClear(&pv);
                }
                ps->Release();
            }
            if (found) break;
            d->Release();
        }
        coll->Release();
    }
    en->Release();
    return found;
}

// 打开共享模式采集流
bool openCapture(WasapiCaptureState *st, const QString &deviceName) {
    st->device = findDeviceByName(deviceName, true);
    if (!st->device) return false;
    HRESULT hr = st->device->Activate(__uuidof(IAudioClient), CLSCTX_ALL, nullptr,
                                      (void **)&st->client);
    if (FAILED(hr)) return false;
    WAVEFORMATEX *fmt = nullptr;
    if (FAILED(st->client->GetMixFormat(&fmt))) return false;
    // 请求 48kHz 单声道 float（共享模式可能返回设备格式，这里声明我们的格式）
    WAVEFORMATEX req = {0};
    req.wFormatTag = WAVE_FORMAT_IEEE_FLOAT;
    req.nChannels = 1;
    req.nSamplesPerSec = kSampleRate;
    req.wBitsPerSample = 32;
    req.nBlockAlign = 4;
    req.nAvgBytesPerSec = kSampleRate * 4;
    REFERENCE_TIME hns = 200000;  // 20ms
    hr = st->client->Initialize(AUDCLNT_SHAREMODE_SHARED,
                                AUDCLNT_STREAMFLAGS_NOPERSIST, hns, 0, &req, nullptr);
    if (FAILED(hr)) {
        // 回退设备格式
        hr = st->client->Initialize(AUDCLNT_SHAREMODE_SHARED,
                                    AUDCLNT_STREAMFLAGS_NOPERSIST, hns, 0, fmt, nullptr);
    }
    CoTaskMemFree(fmt);
    if (FAILED(hr)) return false;
    st->client->GetService(__uuidof(IAudioCaptureClient), (void **)&st->capture);
    if (!st->capture) return false;
    st->client->Start();
    st->running = true;
    return true;
}

// 打开共享模式播放流
bool openRender(WasapiRenderState *st, const QString &deviceName) {
    st->device = findDeviceByName(deviceName, false);
    if (!st->device) return false;
    HRESULT hr = st->device->Activate(__uuidof(IAudioClient), CLSCTX_ALL, nullptr,
                                      (void **)&st->client);
    if (FAILED(hr)) return false;
    WAVEFORMATEX *fmt = nullptr;
    if (FAILED(st->client->GetMixFormat(&fmt))) return false;
    WAVEFORMATEX req = {0};
    req.wFormatTag = WAVE_FORMAT_IEEE_FLOAT;
    req.nChannels = 1;
    req.nSamplesPerSec = kSampleRate;
    req.wBitsPerSample = 32;
    req.nBlockAlign = 4;
    req.nAvgBytesPerSec = kSampleRate * 4;
    REFERENCE_TIME hns = 200000;
    hr = st->client->Initialize(AUDCLNT_SHAREMODE_SHARED,
                                AUDCLNT_STREAMFLAGS_NOPERSIST, hns, 0, &req, nullptr);
    if (FAILED(hr))
        hr = st->client->Initialize(AUDCLNT_SHAREMODE_SHARED,
                                    AUDCLNT_STREAMFLAGS_NOPERSIST, hns, 0, fmt, nullptr);
    CoTaskMemFree(fmt);
    if (FAILED(hr)) return false;
    st->client->GetService(__uuidof(IAudioRenderClient), (void **)&st->render);
    if (!st->render) return false;
    st->client->Start();
    st->running = true;
    return true;
}

// 采集：从 capture client 拉数据到 buffer
void pullCapture(WasapiCaptureState *st) {
    UINT32 framesAvailable = 0;
    BYTE *data = nullptr;
    if (!st->capture) return;
    while (st->capture->GetNextPacketSize(&framesAvailable) == S_OK && framesAvailable > 0) {
        DWORD flags = 0;
        HRESULT hr = st->capture->GetBuffer(&data, &framesAvailable, &flags, nullptr, nullptr);
        if (FAILED(hr)) break;
        if (data) {
            std::lock_guard<std::mutex> lock(st->mutex);
            if (st->mono) {
                for (UINT32 i = 0; i < framesAvailable; ++i) {
                    float v = ((float *)data)[i * st->outCh];
                    st->buffer.push_back(v);
                }
            } else {
                for (UINT32 i = 0; i < framesAvailable * st->outCh; ++i)
                    st->buffer.push_back(((float *)data)[i]);
            }
            if (st->buffer.size() > kSampleRate * 4)  // 4s 上限
                st->buffer.erase(st->buffer.begin(), st->buffer.begin() + kSampleRate);
        }
        st->capture->ReleaseBuffer(framesAvailable);
    }
}

// 播放：从 buffer 写数据到 render client
void pushRender(WasapiRenderState *st) {
    UINT32 framesAvailable = 0;
    st->client->GetBufferSize(&framesAvailable);
    std::lock_guard<std::mutex> lock(st->mutex);
    if (st->buffer.empty()) return;
    UINT32 pad = 0;
    st->client->GetCurrentPadding(&pad);
    UINT32 freeFrames = framesAvailable - pad;
    if (freeFrames == 0) return;
    UINT32 toWrite = std::min((UINT32)st->buffer.size(), freeFrames);
    BYTE *dst = nullptr;
    if (SUCCEEDED(st->render->GetBuffer(toWrite, &dst)) && dst) {
        memcpy(dst, st->buffer.data(), toWrite * sizeof(float));
        st->buffer.erase(st->buffer.begin(), st->buffer.begin() + toWrite);
        st->render->ReleaseBuffer(toWrite, 0);
    }
}

}  // namespace

WasapiBackend::WasapiBackend() = default;

WasapiBackend::~WasapiBackend() { close(); }

bool WasapiBackend::check48k(const QString &deviceName, bool capture) {
    if (deviceName.isEmpty()) return true;
    IMMDevice *dev = findDeviceByName(deviceName, capture);
    if (!dev) return false;
    IAudioClient *cl = nullptr;
    HRESULT hr = dev->Activate(__uuidof(IAudioClient), CLSCTX_ALL, nullptr, (void **)&cl);
    dev->Release();
    if (FAILED(hr)) return false;
    bool ok = false;
    if (cl) {
        WAVEFORMATEX *fmt = nullptr;
        if (SUCCEEDED(cl->GetMixFormat(&fmt))) {
            WAVEFORMATEX req = {0};
            req.wFormatTag = WAVE_FORMAT_IEEE_FLOAT;
            req.nChannels = 1;
            req.nSamplesPerSec = kSampleRate;
            req.wBitsPerSample = 32;
            req.nBlockAlign = 4;
            req.nAvgBytesPerSec = kSampleRate * 4;
            REFERENCE_TIME hns = 200000;
            // 先试我们的 48k float，失败回退设备 MixFormat（共享模式引擎会重采样）
            hr = cl->Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_NOPERSIST,
                                hns, 0, &req, nullptr);
            if (FAILED(hr))
                hr = cl->Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_NOPERSIST,
                                    hns, 0, fmt, nullptr);
            CoTaskMemFree(fmt);
            ok = SUCCEEDED(hr);
        }
        cl->Release();
    }
    return ok;
}

bool WasapiBackend::open(const QString &input, const QString &output, const QString &monitor) {
    close();
    inputId_ = input;
    outputId_ = output;
    if (!input.isEmpty()) {
        cap_ = new WasapiCaptureState;
        if (!openCapture(cap_, input)) {
            lastError_ = QStringLiteral("打开 WASAPI 采集失败");
            close();
            return false;
        }
    }
    if (!output.isEmpty()) {
        rend_ = new WasapiRenderState;
        if (!openRender(rend_, output)) {
            lastError_ = QStringLiteral("打开 WASAPI 播放失败");
            close();
            return false;
        }
    }
    active_ = true;
    return true;
}

void WasapiBackend::close() {
    active_ = false;
    if (cap_) {
        cap_->running = false;
        if (cap_->client) { cap_->client->Stop(); cap_->client->Release(); }
        if (cap_->capture) cap_->capture->Release();
        if (cap_->device) cap_->device->Release();
        delete cap_;
        cap_ = nullptr;
    }
    if (rend_) {
        rend_->running = false;
        if (rend_->client) { rend_->client->Stop(); rend_->client->Release(); }
        if (rend_->render) rend_->render->Release();
        if (rend_->device) rend_->device->Release();
        delete rend_;
        rend_ = nullptr;
    }
}

bool WasapiBackend::active() const { return active_; }

QString WasapiBackend::lastError() const { return lastError_; }

uint32_t WasapiBackend::sampleRate() const { return kSampleRate; }

size_t WasapiBackend::read(float *out, size_t n) {
    if (!cap_ || !cap_->running) return 0;
    pullCapture(cap_);
    std::lock_guard<std::mutex> lock(cap_->mutex);
    size_t r = std::min((size_t)cap_->buffer.size(), n);
    if (r) memcpy(out, cap_->buffer.data(), r * sizeof(float));
    cap_->buffer.erase(cap_->buffer.begin(), cap_->buffer.begin() + r);
    return r;
}

size_t WasapiBackend::readFar(float *out, size_t n) {
    (void)out; (void)n;
    return 0;
}

void WasapiBackend::write(const float *data, size_t n) {
    if (!rend_ || !rend_->running) return;
    {
        std::lock_guard<std::mutex> lock(rend_->mutex);
        rend_->buffer.insert(rend_->buffer.end(), data, data + n);
    }
    pushRender(rend_);
}

bool WasapiBackend::setFar(const QString &sink, bool on) {
    (void)sink; (void)on;
    return false;
}

bool WasapiBackend::setMonitor(const QString &name, bool on) {
    (void)name; (void)on;
    return false;
}

#endif // Q_OS_WIN
