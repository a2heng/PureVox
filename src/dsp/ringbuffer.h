// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_DSP_RINGBUFFER_H
#define PUREVOX_DSP_RINGBUFFER_H

#include <cstddef>
#include <mutex>
#include <vector>

namespace pv {

// 线程安全 FIFO（跨线程高频读写）
class RingBuffer {
public:
    explicit RingBuffer(size_t capacity);
    void write(const float *data, size_t n);
    size_t read(float *dest, size_t n);
    size_t available() const;
    void clear();

private:
    mutable std::mutex mutex_;
    std::vector<float> buffer_;
    size_t capacity_ = 0;
    size_t writePos_ = 0;
    size_t readPos_ = 0;
    size_t count_ = 0;
};

}  // namespace pv

#endif // PUREVOX_DSP_RINGBUFFER_H
