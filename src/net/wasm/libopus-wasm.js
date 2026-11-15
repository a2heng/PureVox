import createLibopusModule from "./generated/libopus.generated.mjs";
export const Application = {
    Voip: 2048,
    Audio: 2049,
    RestrictedLowDelay: 2051,
};
export const Signal = {
    Auto: -1000,
    Voice: 3001,
    Music: 3002,
};
export const Bitrate = {
    Auto: -1000,
    Max: -1,
};
export const Bandwidth = {
    Narrowband: 1101,
    Mediumband: 1102,
    Wideband: 1103,
    Superwideband: 1104,
    Fullband: 1105,
};
export const EncoderCtl = {
    SetApplication: 4000,
    SetBitrate: 4002,
    SetMaxBandwidth: 4004,
    SetVbr: 4006,
    SetBandwidth: 4008,
    SetComplexity: 4010,
    SetInBandFec: 4012,
    SetPacketLossPercent: 4014,
    SetDtx: 4016,
    SetVbrConstraint: 4020,
    SetForceChannels: 4022,
    SetSignal: 4024,
    SetLsbDepth: 4036,
    SetExpertFrameDuration: 4040,
    SetPredictionDisabled: 4042,
    SetPhaseInversionDisabled: 4046,
};
export const DecoderCtl = {
    SetGain: 4034,
    SetPhaseInversionDisabled: 4046,
};
const DEFAULT_CHANNELS = 2;
const DEFAULT_FRAME_DURATION_MS = 20;
const MAX_PACKET_DURATION_MS = 120;
const DEFAULT_MAX_PACKET_BYTES = 4000;
const MIN_I32 = -0x8000_0000;
const MAX_I32 = 0x7fff_ffff;
const DEFAULT_SAMPLE_RATE = 48_000;
const DECODER_INTEGER_CTL_REQUESTS = new Set(Object.values(DecoderCtl));
const ENCODER_INTEGER_CTL_REQUESTS = new Set(Object.values(EncoderCtl));
const ENCODE_FRAME_DURATIONS_MS = [2.5, 5, 10, 20, 40, 60];
const VALID_SAMPLE_RATES = [8000, 12000, 16000, 24000, 48000];
let modulePromise;
export async function loadLibopus() {
    const module = await getModule();
    return { version: module.UTF8ToString(module._oc_get_version_string()) };
}
export async function createEncoder(options = {}) {
    const module = await getModule();
    return new WasmOpusEncoder(module, normalizeEncoderOptions(options));
}
export async function createDecoder(options = {}) {
    const module = await getModule();
    return new WasmOpusDecoder(module, normalizeDecoderOptions(options));
}
export async function getPacketInfo(packet, options = {}) {
    const sampleRate = options.sampleRate ?? DEFAULT_SAMPLE_RATE;
    validateCodecOptions({ channels: DEFAULT_CHANNELS, sampleRate });
    if (packet.byteLength === 0) {
        throw new RangeError("packet must not be empty");
    }
    const module = await getModule();
    const packetPtr = checkedMalloc(module, packet.byteLength);
    try {
        module.HEAPU8.set(packet, packetPtr);
        const decodedSamples = module._oc_packet_validate_decode(packetPtr, packet.byteLength, sampleRate);
        if (decodedSamples < 0) {
            throw createOpusError(module, decodedSamples, "getPacketInfo");
        }
        const frames = module._oc_packet_parse(packetPtr, packet.byteLength);
        if (frames < 0) {
            throw createOpusError(module, frames, "getPacketInfo");
        }
        const samples = module._oc_packet_get_nb_samples(packetPtr, packet.byteLength, sampleRate);
        if (samples < 0) {
            throw createOpusError(module, samples, "getPacketInfo");
        }
        const channels = module._oc_packet_get_nb_channels(packetPtr);
        if (channels !== 1 && channels !== 2) {
            throw new OpusError(channels, `libopus getPacketInfo failed (${channels}): invalid channel count`);
        }
        const bandwidth = module._oc_packet_get_bandwidth(packetPtr);
        if (bandwidth < 0) {
            throw createOpusError(module, bandwidth, "getPacketInfo");
        }
        validateBandwidth(bandwidth, "packet bandwidth");
        return {
            bandwidth: bandwidth,
            channels,
            durationMs: (samples / sampleRate) * 1000,
            frames,
            samples,
            samplesPerFrame: module._oc_packet_get_samples_per_frame(packetPtr, sampleRate),
            sampleRate,
        };
    }
    finally {
        module._free(packetPtr);
    }
}
class WasmBuffer {
    module;
    #bytes = 0;
    #ptr = 0;
    constructor(module) {
        this.module = module;
    }
    ensure(requiredBytes) {
        if (this.#ptr !== 0 && this.#bytes >= requiredBytes) {
            return this.#ptr;
        }
        const nextPtr = checkedMalloc(this.module, requiredBytes);
        this.free();
        this.#ptr = nextPtr;
        this.#bytes = requiredBytes;
        return this.#ptr;
    }
    free() {
        if (this.#ptr !== 0) {
            this.module._free(this.#ptr);
        }
        this.#ptr = 0;
        this.#bytes = 0;
    }
}
class WasmOpusEncoder {
    application;
    channels;
    frameSize;
    sampleRate;
    #freed = false;
    #module;
    #packet;
    #pcm;
    #ptr;
    constructor(module, options) {
        this.#module = module;
        this.#packet = new WasmBuffer(module);
        this.#pcm = new WasmBuffer(module);
        this.application = options.application;
        this.channels = options.channels;
        this.frameSize = options.frameSize;
        this.sampleRate = options.sampleRate;
        const errorPtr = checkedMalloc(module, 4);
        try {
            const ptr = module._oc_create_encoder(options.sampleRate, options.channels, options.application, errorPtr);
            const error = module.HEAP32[errorPtr >> 2] ?? 0;
            if (!ptr || error !== 0) {
                throw createOpusError(module, error, "createEncoder");
            }
            this.#ptr = ptr;
        }
        finally {
            module._free(errorPtr);
        }
        let configured = false;
        try {
            this.setBitrate(options.bitrate);
            this.setComplexity(options.complexity);
            this.setDtx(options.dtx);
            this.setFec(options.fec);
            if (options.maxBandwidth !== undefined) {
                this.setMaxBandwidth(options.maxBandwidth);
            }
            this.setPacketLossPercent(options.packetLossPercent);
            this.setSignal(options.signal);
            if (options.vbr !== undefined) {
                this.setVbr(options.vbr);
            }
            if (options.vbrConstraint !== undefined) {
                this.setVbrConstraint(options.vbrConstraint);
            }
            configured = true;
        }
        finally {
            if (!configured) {
                this.free();
            }
        }
    }
    encode(pcm, options = {}) {
        this.#assertLive();
        const frameSize = options.frameSize ?? this.frameSize;
        validateEncodeFrameSize(frameSize, this.sampleRate, "frameSize");
        const pcmBytes = toUint8Array(pcm);
        const expectedBytes = frameSize * this.channels * 2;
        if (pcmBytes.byteLength !== expectedBytes) {
            throw new RangeError(`PCM frame has ${pcmBytes.byteLength} bytes; expected ${expectedBytes} for ${frameSize} samples and ${this.channels} channel(s)`);
        }
        const maxPacketBytes = options.maxPacketBytes ?? DEFAULT_MAX_PACKET_BYTES;
        validateIntegerRange(maxPacketBytes, 1, MAX_I32, "maxPacketBytes");
        const pcmPtr = this.#pcm.ensure(pcmBytes.byteLength);
        const packetPtr = this.#packet.ensure(maxPacketBytes);
        this.#module.HEAPU8.set(pcmBytes, pcmPtr);
        const encodedBytes = this.#module._oc_encode(this.#ptr, pcmPtr, frameSize, packetPtr, maxPacketBytes);
        if (encodedBytes < 0) {
            throw createOpusError(this.#module, encodedBytes, "encode");
        }
        return this.#module.HEAPU8.slice(packetPtr, packetPtr + encodedBytes);
    }
    encodeFloat(pcm, options = {}) {
        this.#assertLive();
        const frameSize = options.frameSize ?? this.frameSize;
        validateEncodeFrameSize(frameSize, this.sampleRate, "frameSize");
        const expectedSamples = frameSize * this.channels;
        if (pcm.length !== expectedSamples) {
            throw new RangeError(`Float32 PCM frame has ${pcm.length} samples; expected ${expectedSamples} for ${frameSize} samples and ${this.channels} channel(s)`);
        }
        const maxPacketBytes = options.maxPacketBytes ?? DEFAULT_MAX_PACKET_BYTES;
        validateIntegerRange(maxPacketBytes, 1, MAX_I32, "maxPacketBytes");
        const pcmPtr = this.#pcm.ensure(pcm.byteLength);
        const packetPtr = this.#packet.ensure(maxPacketBytes);
        this.#module.HEAPF32.set(pcm, pcmPtr >> 2);
        const encodedBytes = this.#module._oc_encode_float(this.#ptr, pcmPtr, frameSize, packetPtr, maxPacketBytes);
        if (encodedBytes < 0) {
            throw createOpusError(this.#module, encodedBytes, "encodeFloat");
        }
        return this.#module.HEAPU8.slice(packetPtr, packetPtr + encodedBytes);
    }
    encodeFrames(frames, options = {}) {
        return frames.map((frame) => this.encode(frame, options));
    }
    encodeFloatFrames(frames, options = {}) {
        return frames.map((frame) => this.encodeFloat(frame, options));
    }
    encoderCtl(request, value) {
        this.#assertLive();
        validateInteger(request, "request");
        validateIntegerRange(value, MIN_I32, MAX_I32, "value");
        if (!ENCODER_INTEGER_CTL_REQUESTS.has(request)) {
            throw new RangeError("encoderCtl only supports integer setter requests");
        }
        this.#check(this.#module._oc_encoder_ctl(this.#ptr, request, value), "encoderCtl");
    }
    setBitrate(bitrate) {
        this.encoderCtl(EncoderCtl.SetBitrate, normalizeBitrate(bitrate));
    }
    getBitrate() {
        this.#assertLive();
        const bitrate = this.#module._oc_encoder_ctl_get_bitrate(this.#ptr);
        if (bitrate < 0) {
            throw createOpusError(this.#module, bitrate, "getBitrate");
        }
        return bitrate;
    }
    getLookahead() {
        this.#assertLive();
        const lookahead = this.#module._oc_encoder_ctl_get_lookahead(this.#ptr);
        if (lookahead < 0) {
            throw createOpusError(this.#module, lookahead, "getLookahead");
        }
        return lookahead;
    }
    getInDtx() {
        this.#assertLive();
        const inDtx = this.#module._oc_encoder_ctl_get_in_dtx(this.#ptr);
        if (inDtx < 0) {
            throw createOpusError(this.#module, inDtx, "getInDtx");
        }
        return inDtx !== 0;
    }
    setComplexity(complexity) {
        validateIntegerRange(complexity, 0, 10, "complexity");
        this.encoderCtl(EncoderCtl.SetComplexity, complexity);
    }
    setDtx(enabled) {
        this.encoderCtl(EncoderCtl.SetDtx, enabled ? 1 : 0);
    }
    setFec(enabled) {
        this.encoderCtl(EncoderCtl.SetInBandFec, enabled ? 1 : 0);
    }
    setMaxBandwidth(bandwidth) {
        validateBandwidth(bandwidth, "maxBandwidth");
        this.encoderCtl(EncoderCtl.SetMaxBandwidth, bandwidth);
    }
    setPacketLossPercent(percentage) {
        validateIntegerRange(percentage, 0, 100, "packetLossPercent");
        this.encoderCtl(EncoderCtl.SetPacketLossPercent, percentage);
    }
    setSignal(signal) {
        if (!Object.values(Signal).includes(signal)) {
            throw new RangeError("signal must be Signal.Auto, Signal.Voice, or Signal.Music");
        }
        this.encoderCtl(EncoderCtl.SetSignal, signal);
    }
    setVbr(enabled) {
        this.encoderCtl(EncoderCtl.SetVbr, enabled ? 1 : 0);
    }
    setVbrConstraint(enabled) {
        this.encoderCtl(EncoderCtl.SetVbrConstraint, enabled ? 1 : 0);
    }
    free() {
        if (this.#freed) {
            return;
        }
        this.#packet.free();
        this.#pcm.free();
        this.#module._oc_destroy_encoder(this.#ptr);
        this.#freed = true;
    }
    [Symbol.dispose]() {
        this.free();
    }
    #assertLive() {
        if (this.#freed) {
            throw new Error("OpusEncoder has been freed");
        }
    }
    #check(code, operation) {
        if (code < 0) {
            throw createOpusError(this.#module, code, operation);
        }
    }
}
class WasmOpusDecoder {
    channels;
    maxFrameSize;
    sampleRate;
    #freed = false;
    #module;
    #packet;
    #pcm;
    #ptr;
    constructor(module, options) {
        this.#module = module;
        this.#packet = new WasmBuffer(module);
        this.#pcm = new WasmBuffer(module);
        this.channels = options.channels;
        this.maxFrameSize = options.maxFrameSize;
        this.sampleRate = options.sampleRate;
        const errorPtr = checkedMalloc(module, 4);
        try {
            const ptr = module._oc_create_decoder(options.sampleRate, options.channels, errorPtr);
            const error = module.HEAP32[errorPtr >> 2] ?? 0;
            if (!ptr || error !== 0) {
                throw createOpusError(module, error, "createDecoder");
            }
            this.#ptr = ptr;
        }
        finally {
            module._free(errorPtr);
        }
    }
    decode(packet, options = {}) {
        this.#assertLive();
        const frameSize = this.#resolveDecodeFrameSize(packet, options);
        const pcmBytes = frameSize * this.channels * 2;
        const pcmPtr = this.#pcm.ensure(pcmBytes);
        const { packetLength, packetPtr } = this.#copyPacket(packet, options.decodeFec);
        const decodedSamples = this.#module._oc_decode(this.#ptr, packetPtr, packetLength, pcmPtr, frameSize, options.decodeFec ? 1 : 0);
        if (decodedSamples < 0) {
            throw createOpusError(this.#module, decodedSamples, packet === null ? "decodePacketLoss" : "decode");
        }
        const sampleCount = decodedSamples * this.channels;
        return this.#module.HEAP16.slice(pcmPtr >> 1, (pcmPtr >> 1) + sampleCount);
    }
    decodeFloat(packet, options = {}) {
        this.#assertLive();
        const frameSize = this.#resolveDecodeFrameSize(packet, options);
        const pcmBytes = frameSize * this.channels * 4;
        const pcmPtr = this.#pcm.ensure(pcmBytes);
        const { packetLength, packetPtr } = this.#copyPacket(packet, options.decodeFec);
        const decodedSamples = this.#module._oc_decode_float(this.#ptr, packetPtr, packetLength, pcmPtr, frameSize, options.decodeFec ? 1 : 0);
        if (decodedSamples < 0) {
            throw createOpusError(this.#module, decodedSamples, packet === null ? "decodePacketLossFloat" : "decodeFloat");
        }
        const sampleCount = decodedSamples * this.channels;
        return this.#module.HEAPF32.slice(pcmPtr >> 2, (pcmPtr >> 2) + sampleCount);
    }
    decodeFrames(packets, options = {}) {
        return packets.map((packet) => this.decode(packet, options));
    }
    decodeFloatFrames(packets, options = {}) {
        return packets.map((packet) => this.decodeFloat(packet, options));
    }
    decodePacketLoss(frameSize = samplesForDuration(this.sampleRate, DEFAULT_FRAME_DURATION_MS)) {
        return this.decode(null, { frameSize });
    }
    decodePacketLossFloat(frameSize = samplesForDuration(this.sampleRate, DEFAULT_FRAME_DURATION_MS)) {
        return this.decodeFloat(null, { frameSize });
    }
    decoderCtl(request, value) {
        this.#assertLive();
        validateInteger(request, "request");
        validateIntegerRange(value, MIN_I32, MAX_I32, "value");
        if (!DECODER_INTEGER_CTL_REQUESTS.has(request)) {
            throw new RangeError("decoderCtl only supports integer setter requests");
        }
        const code = this.#module._oc_decoder_ctl(this.#ptr, request, value);
        if (code < 0) {
            throw createOpusError(this.#module, code, "decoderCtl");
        }
    }
    free() {
        if (this.#freed) {
            return;
        }
        this.#packet.free();
        this.#pcm.free();
        this.#module._oc_destroy_decoder(this.#ptr);
        this.#freed = true;
    }
    [Symbol.dispose]() {
        this.free();
    }
    #assertLive() {
        if (this.#freed) {
            throw new Error("OpusDecoder has been freed");
        }
    }
    #copyPacket(packet, decodeFec) {
        if (packet === null) {
            if (decodeFec) {
                throw new RangeError("decodeFec requires a packet");
            }
            return { packetLength: 0, packetPtr: 0 };
        }
        if (packet.byteLength === 0) {
            throw new RangeError("packet must not be empty; use null or decodePacketLoss() for PLC");
        }
        const packetPtr = this.#packet.ensure(packet.byteLength);
        this.#module.HEAPU8.set(packet, packetPtr);
        return { packetLength: packet.byteLength, packetPtr };
    }
    #resolveDecodeFrameSize(packet, options) {
        const frameSize = packet === null || options.decodeFec
            ? (options.frameSize ?? options.maxFrameSize ?? samplesForDuration(this.sampleRate, DEFAULT_FRAME_DURATION_MS))
            : (options.maxFrameSize ?? this.maxFrameSize);
        if (packet === null || options.decodeFec) {
            validatePlcFrameSize(frameSize, this.sampleRate, "frameSize");
            return frameSize;
        }
        validateDecodeCapacity(frameSize, this.sampleRate, "maxFrameSize");
        return frameSize;
    }
}
export class OpusError extends Error {
    code;
    codeName;
    operation;
    constructor(code, message, operation) {
        super(message);
        this.name = "OpusError";
        this.code = code;
        this.codeName = resolveOpusErrorCodeName(code);
        this.operation = operation;
    }
}
export const OpusErrorCode = {
    BadArg: -1,
    BufferTooSmall: -2,
    InternalError: -3,
    InvalidPacket: -4,
    Unimplemented: -5,
    InvalidState: -6,
    AllocFail: -7,
};
export function isOpusError(error) {
    if (error instanceof OpusError) {
        return true;
    }
    const candidate = error;
    return (Boolean(error) &&
        typeof error === "object" &&
        candidate.name === "OpusError" &&
        typeof candidate.message === "string" &&
        typeof candidate.code === "number" &&
        (typeof candidate.codeName === "string" || candidate.codeName === undefined) &&
        (typeof candidate.operation === "string" || candidate.operation === undefined));
}
async function getModule() {
    modulePromise ??= createLibopusModule();
    return await modulePromise;
}
function resolveOpusErrorCodeName(code) {
    for (const [name, value] of Object.entries(OpusErrorCode)) {
        if (value === code) {
            return name;
        }
    }
    return undefined;
}
function createOpusError(module, code, operation) {
    const message = module.UTF8ToString(module._oc_strerror(code));
    return new OpusError(code, `libopus ${operation} failed (${code}): ${message}`, operation);
}
function toUint8Array(input) {
    return input instanceof Uint8Array
        ? input
        : new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
}
function normalizeEncoderOptions(options) {
    const sampleRate = options.sampleRate ?? DEFAULT_SAMPLE_RATE;
    const channels = options.channels ?? DEFAULT_CHANNELS;
    validateCodecOptions({ channels, sampleRate });
    const application = options.application ?? Application.Audio;
    if (!Object.values(Application).includes(application)) {
        throw new RangeError("application must be Application.Voip, Application.Audio, or Application.RestrictedLowDelay");
    }
    const frameSize = options.frameSize ?? samplesForDuration(sampleRate, DEFAULT_FRAME_DURATION_MS);
    validateEncodeFrameSize(frameSize, sampleRate, "frameSize");
    if (options.maxBandwidth !== undefined) {
        validateBandwidth(options.maxBandwidth, "maxBandwidth");
    }
    const complexity = options.complexity ?? 10;
    validateIntegerRange(complexity, 0, 10, "complexity");
    const packetLossPercent = options.packetLossPercent ?? 0;
    validateIntegerRange(packetLossPercent, 0, 100, "packetLossPercent");
    const signal = options.signal ?? Signal.Auto;
    if (!Object.values(Signal).includes(signal)) {
        throw new RangeError("signal must be Signal.Auto, Signal.Voice, or Signal.Music");
    }
    return {
        application,
        bitrate: normalizeBitrate(options.bitrate ?? 64_000),
        channels,
        complexity,
        dtx: options.dtx ?? false,
        fec: options.fec ?? false,
        frameSize,
        maxBandwidth: options.maxBandwidth,
        packetLossPercent,
        sampleRate,
        signal,
        vbr: options.vbr,
        vbrConstraint: options.vbrConstraint,
    };
}
function normalizeDecoderOptions(options) {
    const sampleRate = options.sampleRate ?? DEFAULT_SAMPLE_RATE;
    const channels = options.channels ?? DEFAULT_CHANNELS;
    validateCodecOptions({ channels, sampleRate });
    const maxFrameSize = options.maxFrameSize ?? samplesForDuration(sampleRate, MAX_PACKET_DURATION_MS);
    validateDecodeCapacity(maxFrameSize, sampleRate, "maxFrameSize");
    return { channels, maxFrameSize, sampleRate };
}
function samplesForDuration(sampleRate, durationMs) {
    return (sampleRate / 1000) * durationMs;
}
function validateCodecOptions(options) {
    if (!VALID_SAMPLE_RATES.includes(options.sampleRate)) {
        throw new RangeError("sampleRate must be 8000, 12000, 16000, 24000, or 48000");
    }
    if (options.channels !== 1 && options.channels !== 2) {
        throw new RangeError("channels must be 1 or 2");
    }
}
function normalizeBitrate(bitrate) {
    if (bitrate === "auto") {
        return Bitrate.Auto;
    }
    if (bitrate === "max") {
        return Bitrate.Max;
    }
    if (bitrate === Bitrate.Auto || bitrate === Bitrate.Max) {
        return bitrate;
    }
    validateIntegerRange(bitrate, 1, MAX_I32, "bitrate");
    return bitrate;
}
function validateBandwidth(bandwidth, name) {
    if (!Object.values(Bandwidth).includes(bandwidth)) {
        throw new RangeError(`${name} must be Bandwidth.Narrowband, Bandwidth.Mediumband, Bandwidth.Wideband, Bandwidth.Superwideband, or Bandwidth.Fullband`);
    }
}
function validateEncodeFrameSize(frameSize, sampleRate, name) {
    validateFrameSizeForDurations(frameSize, sampleRate, name, ENCODE_FRAME_DURATIONS_MS);
}
function validateDecodeCapacity(frameSize, sampleRate, name) {
    const maxFrameSize = samplesForDuration(sampleRate, MAX_PACKET_DURATION_MS);
    if (!Number.isInteger(frameSize) || frameSize <= 0 || frameSize > maxFrameSize) {
        throw new RangeError(`${name} must be an integer from 1 to ${maxFrameSize} samples at ${sampleRate} Hz`);
    }
}
function validatePlcFrameSize(frameSize, sampleRate, name) {
    const minFrameSize = samplesForDuration(sampleRate, 2.5);
    const maxFrameSize = samplesForDuration(sampleRate, MAX_PACKET_DURATION_MS);
    if (!Number.isInteger(frameSize) ||
        frameSize < minFrameSize ||
        frameSize > maxFrameSize ||
        frameSize % minFrameSize !== 0) {
        throw new RangeError(`${name} must be a multiple of ${minFrameSize} samples from ${minFrameSize} to ${maxFrameSize} at ${sampleRate} Hz`);
    }
}
function validateFrameSizeForDurations(frameSize, sampleRate, name, durationsMs) {
    const validFrameSizes = durationsMs.map((durationMs) => samplesForDuration(sampleRate, durationMs));
    if (!Number.isInteger(frameSize) || !validFrameSizes.includes(frameSize)) {
        throw new RangeError(`${name} must be one of ${validFrameSizes.join(", ")} samples at ${sampleRate} Hz`);
    }
}
function validateInteger(value, name) {
    if (!Number.isInteger(value)) {
        throw new RangeError(`${name} must be an integer`);
    }
}
function validateIntegerRange(value, min, max, name) {
    if (!Number.isInteger(value) || value < min || value > max) {
        throw new RangeError(`${name} must be an integer from ${min} to ${max}`);
    }
}
function checkedMalloc(module, bytes) {
    validateIntegerRange(bytes, 1, MAX_I32, "malloc size");
    const ptr = module._malloc(bytes);
    if (ptr === 0) {
        throw new Error(`WASM malloc failed for ${bytes} bytes`);
    }
    return ptr;
}
