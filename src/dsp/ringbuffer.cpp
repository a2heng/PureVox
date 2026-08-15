// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "ringbuffer.h"

#include <cstring>

namespace pv {

RingBuffer::RingBuffer(size_t capacity)
    : buffer_(capacity ? capacity : 1, 0.0f), capacity_(capacity) {}

void RingBuffer::write(const float *data, size_t n) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (n > capacity_) n = capacity_;
    for (size_t i = 0; i < n; ++i) {
        buffer_[writePos_++] = data[i];
        if (writePos_ >= capacity_) writePos_ = 0;
        if (count_ < capacity_) count_++;
        else readPos_ = writePos_;
    }
}

size_t RingBuffer::read(float *dest, size_t n) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (n > count_) n = count_;
    for (size_t i = 0; i < n; ++i) {
        dest[i] = buffer_[readPos_++];
        if (readPos_ >= capacity_) readPos_ = 0;
        count_--;
    }
    return n;
}

size_t RingBuffer::available() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return count_;
}

void RingBuffer::clear() {
    std::lock_guard<std::mutex> lock(mutex_);
    writePos_ = readPos_ = count_ = 0;
    std::memset(buffer_.data(), 0, capacity_ * sizeof(float));
}

}  // namespace pv
