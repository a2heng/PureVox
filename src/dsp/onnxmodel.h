// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_ONNXMODEL_H
#define PUREVOX_DSP_ONNXMODEL_H

#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

struct OrtApi;
struct OrtEnv;
struct OrtSessionOptions;
struct OrtSession;
struct OrtMemoryInfo;
struct OrtAllocator;
struct OrtValue;

namespace pv {

// ONNX Runtime C API 的 RAII 封装：单个 ONNX 模型会话。
class OnnxModel {
public:
    OnnxModel() = default;
    ~OnnxModel();

    OnnxModel(const OnnxModel &) = delete;
    OnnxModel &operator=(const OnnxModel &) = delete;

    // 打开模型；成功返回 true。backend_effective/reason 报告推理后端。
    bool open(const std::string &name, const std::string &path);
    void close();

    bool valid() const { return session_ != nullptr; }

    // 输入/输出张量构造与运行
    OrtValue *makeTensor(float *data, size_t count, const int64_t *shape, size_t ndim);
    float *tensorData(OrtValue *v);
    size_t tensorElems(OrtValue *v);
    bool run(const char *const *inNames, OrtValue *const *in, size_t nin,
             const char *const *outNames, OrtValue **out, size_t nout);    void releaseValue(OrtValue *v);

    // 按名查输入总元素数（fallback 默认值）
    size_t inputTotal(const char *name, size_t fallback) const;
    const char *inputName(size_t i) const { return i < nInputs_ ? inputNames_[i] : nullptr; }
    const char *outputName(size_t i) const { return i < nOutputs_ ? outputNames_[i] : nullptr; }
    size_t nInputs() const { return nInputs_; }
    size_t nOutputs() const { return nOutputs_; }

    int backendEffective() const { return backendEffective_; }
    int backendReason() const { return backendReason_; }

private:
    const OrtApi *api_ = nullptr;
    OrtEnv *env_ = nullptr;
    OrtSessionOptions *opts_ = nullptr;
    OrtSession *session_ = nullptr;
    OrtMemoryInfo *meminfo_ = nullptr;
    OrtAllocator *allocator_ = nullptr;
    char **inputNames_ = nullptr;
    char **outputNames_ = nullptr;
    size_t nInputs_ = 0;
    size_t nOutputs_ = 0;
    int64_t **inputShapes_ = nullptr;
    size_t *inputNdims_ = nullptr;
    int backendEffective_ = 0;
    int backendReason_ = 0;
};

}  // namespace pv

#endif // PUREVOX_DSP_ONNXMODEL_H
