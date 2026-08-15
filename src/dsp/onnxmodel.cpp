// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "onnxmodel.h"

#include <onnxruntime_c_api.h>

#include <cstdio>
#include <cstdlib>
#include <cstring>

namespace pv {

namespace {

// 平台相关：Windows 用宽字符路径创建会话
int createSessionPath(const OrtApi *api, OrtEnv *env, const char *path,
                      OrtSessionOptions *opts, OrtSession **out) {
    OrtStatus *st = api->CreateSession(env, path, opts, out);
    if (st) {
        const char *msg = api->GetErrorMessage(st);
        std::fprintf(stderr, "pv: onnxruntime error: %s\n", msg ? msg : "unknown");
        api->ReleaseStatus(st);
        return 0;
    }
    return 1;
}

bool ok(const OrtApi *api, OrtStatus *st) {
    if (st) {
        const char *msg = api->GetErrorMessage(st);
        std::fprintf(stderr, "pv: onnxruntime error: %s\n", msg ? msg : "unknown");
        api->ReleaseStatus(st);
        return false;
    }
    return true;
}

int cpuSupportsAvx() {
#if defined(__GNUC__) || defined(__clang__)
    return __builtin_cpu_supports("avx") ? 1 : 0;
#else
    return 0;
#endif
}

}  // namespace

OnnxModel::~OnnxModel() { close(); }

bool OnnxModel::open(const std::string &name, const std::string &path) {
    close();
    api_ = OrtGetApiBase()->GetApi(ORT_API_VERSION);
    if (!api_) return false;

    if (!ok(api_, api_->CreateEnv(ORT_LOGGING_LEVEL_WARNING, name.c_str(), &env_))) return false;
    if (!ok(api_, api_->CreateSessionOptions(&opts_))) return false;
    if (!ok(api_, api_->SetIntraOpNumThreads(opts_, 1))) return false;
    if (!ok(api_, api_->SetInterOpNumThreads(opts_, 1))) return false;
    if (!ok(api_, api_->SetSessionExecutionMode(opts_, ORT_SEQUENTIAL))) return false;
    if (!ok(api_, api_->SetSessionGraphOptimizationLevel(opts_, ORT_ENABLE_BASIC))) return false;
    if (!ok(api_, api_->GetAllocatorWithDefaultOptions(&allocator_))) return false;
    if (!ok(api_, api_->CreateMemoryInfo("Cpu", OrtArenaAllocator, 0, OrtMemTypeDefault,
                                         &meminfo_))) return false;
    if (!createSessionPath(api_, env_, path.c_str(), opts_, &session_)) return false;

    backendEffective_ = cpuSupportsAvx() ? 0 : 1;  // AIMIC_BACKEND_AVX / SSE
    backendReason_ = 0;

    size_t ni = 0, no = 0;
    if (!ok(api_, api_->SessionGetInputCount(session_, &ni))) return false;
    if (!ok(api_, api_->SessionGetOutputCount(session_, &no))) return false;
    nInputs_ = ni;
    nOutputs_ = no;
    inputNames_ = (char **)calloc(ni ? ni : 1, sizeof(char *));
    outputNames_ = (char **)calloc(no ? no : 1, sizeof(char *));
    inputShapes_ = (int64_t **)calloc(ni ? ni : 1, sizeof(int64_t *));
    inputNdims_ = (size_t *)calloc(ni ? ni : 1, sizeof(size_t));
    if (!inputNames_ || !outputNames_ || !inputShapes_ || !inputNdims_) return false;

    for (size_t i = 0; i < ni; ++i) {
        if (!ok(api_, api_->SessionGetInputName(session_, i, allocator_, &inputNames_[i])))
            return false;
        OrtTypeInfo *typeinfo = nullptr;
        if (!ok(api_, api_->SessionGetInputTypeInfo(session_, i, &typeinfo))) return false;
        const OrtTensorTypeAndShapeInfo *tinfo = nullptr;
        if (!ok(api_, api_->CastTypeInfoToTensorInfo(typeinfo, &tinfo))) {
            api_->ReleaseTypeInfo(typeinfo);
            return false;
        }
        size_t nd = 0;
        api_->GetDimensionsCount(tinfo, &nd);
        inputNdims_[i] = nd;
        inputShapes_[i] = (int64_t *)calloc(nd ? nd : 1, sizeof(int64_t));
        if (nd > 0) api_->GetDimensions(tinfo, inputShapes_[i], nd);
        api_->ReleaseTypeInfo(typeinfo);
    }
    for (size_t i = 0; i < no; ++i) {
        if (!ok(api_, api_->SessionGetOutputName(session_, i, allocator_, &outputNames_[i])))
            return false;
    }
    return true;
}

void OnnxModel::close() {
    if (!api_) return;
    if (inputNames_) {
        for (size_t i = 0; i < nInputs_; ++i)
            if (inputNames_[i]) api_->AllocatorFree(allocator_, inputNames_[i]);
    }
    if (outputNames_) {
        for (size_t i = 0; i < nOutputs_; ++i)
            if (outputNames_[i]) api_->AllocatorFree(allocator_, outputNames_[i]);
    }
    if (inputShapes_) {
        for (size_t i = 0; i < nInputs_; ++i) free(inputShapes_[i]);
    }
    free(inputNames_);
    free(outputNames_);
    free(inputShapes_);
    free(inputNdims_);
    inputNames_ = outputNames_ = nullptr;
    inputShapes_ = nullptr;
    inputNdims_ = nullptr;
    if (session_) { api_->ReleaseSession(session_); session_ = nullptr; }
    if (opts_) { api_->ReleaseSessionOptions(opts_); opts_ = nullptr; }
    if (meminfo_) { api_->ReleaseMemoryInfo(meminfo_); meminfo_ = nullptr; }
    if (env_) { api_->ReleaseEnv(env_); env_ = nullptr; }
    nInputs_ = nOutputs_ = 0;
}

size_t OnnxModel::inputTotal(const char *name, size_t fallback) const {
    for (size_t i = 0; i < nInputs_; ++i) {
        if (inputNames_[i] && std::strcmp(inputNames_[i], name) == 0) {
            size_t total = 1;
            for (size_t j = 0; j < inputNdims_[i]; ++j) {
                int64_t d = inputShapes_[i][j];
                if (d <= 0) return fallback;
                total *= (size_t)d;
            }
            return total;
        }
    }
    return fallback;
}

OrtValue *OnnxModel::makeTensor(float *data, size_t count, const int64_t *shape, size_t ndim) {
    OrtValue *v = nullptr;
    OrtStatus *st = api_->CreateTensorWithDataAsOrtValue(
        meminfo_, data, count * sizeof(float), shape, ndim, ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT, &v);
    if (st) {
        std::fprintf(stderr, "pv: CreateTensor failed: %s\n", api_->GetErrorMessage(st));
        api_->ReleaseStatus(st);
        return nullptr;
    }
    return v;
}

float *OnnxModel::tensorData(OrtValue *v) {
    void *p = nullptr;
    if (api_->GetTensorMutableData(v, &p)) return nullptr;
    return (float *)p;
}

size_t OnnxModel::tensorElems(OrtValue *v) {
    OrtTensorTypeAndShapeInfo *info = nullptr;
    if (api_->GetTensorTypeAndShape(v, &info)) return 0;
    if (!info) return 0;
    size_t nd = 0;
    api_->GetDimensionsCount(info, &nd);
    int64_t dims[16];
    if (nd > 16) nd = 16;
    if (nd > 0) api_->GetDimensions(info, dims, nd);
    size_t total = 1;
    for (size_t i = 0; i < nd; ++i) {
        if (dims[i] <= 0) { total = 0; break; }
        total *= (size_t)dims[i];
    }
    api_->ReleaseTensorTypeAndShapeInfo(info);
    return total;
}

bool OnnxModel::run(const char *const *inNames, OrtValue *const *in, size_t nin,
                    const char *const *outNames, OrtValue **out, size_t nout) {
    // 用模型自身的输入/输出名（调用方可传 nullptr 表示用默认名序）
    std::vector<const char *> inN(nin ? nin : 1);
    std::vector<const char *> outN(nout ? nout : 1);
    for (size_t i = 0; i < nin; ++i) inN[i] = inNames ? inNames[i] : inputNames_[i];
    for (size_t i = 0; i < nout; ++i) outN[i] = outNames ? outNames[i] : outputNames_[i];
    OrtStatus *st = api_->Run(session_, nullptr, inN.data(), (const OrtValue *const *)in, nin,
                              outN.data(), nout, out);
    if (st) {
        std::fprintf(stderr, "pv: Run failed: %s\n", api_->GetErrorMessage(st));
        api_->ReleaseStatus(st);
        return false;
    }
    return true;
}

void OnnxModel::releaseValue(OrtValue *v) {
    if (v) api_->ReleaseValue(v);
}

}  // namespace pv
