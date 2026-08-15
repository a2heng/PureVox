/* PureVox — AI 麦克风降噪工具
 * Copyright (C) 2024-2026 a2heng <752848283@qq.com>
 * GPL-3.0-or-later.  See LICENSE for details.
 * The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).
 *
 * pvbridge.h — pvpipe（PipeWire 桥）的公共 C API 声明。
 * 实现见 pipewire_client.c。C++ 侧仅以不透明指针调用，不需要完整结构。
 */

#ifndef PUREVOX_PVBRIDGE_H
#define PUREVOX_PVBRIDGE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PwBridge PwBridge;

PwBridge*   pvb_new(void);
void        pvb_free(PwBridge* self);
int         pvb_open(PwBridge* self, const char* input_name, const char* output_name,
                     const char* monitor_name);
void        pvb_close(PwBridge* self);
int         pvb_active(const PwBridge* self);
const char* pvb_last_error(const PwBridge* self);
uint32_t    pvb_sample_rate(const PwBridge* self);
uint32_t    pvb_buffer_size(const PwBridge* self);
int         pvb_set_monitor(PwBridge* self, const char* monitor_name, int enabled);
int         pvb_set_far(PwBridge* self, const char* sink_name, int enabled);
size_t      pvb_read(PwBridge* self, float* out, size_t n);
size_t      pvb_read_far(PwBridge* self, float* out, size_t n);
void        pvb_write(PwBridge* self, const float* data, size_t n);

#ifdef __cplusplus
}
#endif

#endif /* PUREVOX_PVBRIDGE_H */
