/* PureVox — AI 麦克风降噪工具
 * Copyright (C) 2024-2026 a2heng <752848283@qq.com>
 * GPL-3.0-or-later.  See LICENSE for details.
 * The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).
 *
 * alsbridge.h — pvalsa（ALSA 桥）的公共 C API 声明。
 * 实现见 alsa_client.c。C++ 侧仅以不透明指针调用。
 */

#ifndef PUREVOX_ALSBRIDGE_H
#define PUREVOX_ALSBRIDGE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct AlsaBridge AlsaBridge;

AlsaBridge* als_new(void);
void        als_free(AlsaBridge* self);
int         als_open(AlsaBridge* self, const char* input_name, const char* output_name,
                     const char* monitor_name);
void        als_close(AlsaBridge* self);
int         als_active(const AlsaBridge* self);
const char* als_last_error(const AlsaBridge* self);
uint32_t    als_sample_rate(const AlsaBridge* self);
uint32_t    als_buffer_size(const AlsaBridge* self);
int         als_set_monitor(AlsaBridge* self, const char* monitor_name, int enabled);
int         als_set_far(AlsaBridge* self, const char* far_name, int enabled);
size_t      als_read(AlsaBridge* self, float* out, size_t n);
size_t      als_read_far(AlsaBridge* self, float* out, size_t n);
void        als_write(AlsaBridge* self, const float* data, size_t n);

#ifdef __cplusplus
}
#endif

#endif /* PUREVOX_ALSBRIDGE_H */
