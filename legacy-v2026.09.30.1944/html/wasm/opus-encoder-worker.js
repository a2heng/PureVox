self.OPUS_SCRIPT_LOCATION = self.location.href.substring(0, self.location.href.lastIndexOf('/') + 1);

importScripts('libopus-encoder.wasm.min.js');

const OPUS_APPLICATION_VOIP = 2048;
const OPUS_SET_BITRATE = 4002;
const SAMPLE_RATE = 48000;
const FRAME_SIZE = 480;   // 10ms @ 48kHz（与模型 hop 一致）
const MAX_PACKET = 4000;

let encoder = null;
let opusPtr = null;
let ready = false;
let pending = [];

function onWasmReady() {
    encoder = OpusEncoderLib._opus_encoder_create(SAMPLE_RATE, 1, OPUS_APPLICATION_VOIP);
    OpusEncoderLib._opus_encoder_ctl(encoder, OPUS_SET_BITRATE, 32000);
    opusPtr = OpusEncoderLib._malloc(MAX_PACKET);
    ready = true;
    for (const msg of pending) handle(msg);
    pending = [];
}

if (OpusEncoderLib.isReady) {
    onWasmReady();
} else {
    OpusEncoderLib.onready = onWasmReady;
}

function handle(msg) {
    if (msg.type === 'encode') {
        const pcmData = msg.data;
        const pcmPtr = OpusEncoderLib._malloc(pcmData.length * 4);
        OpusEncoderLib.HEAPF32.set(pcmData, pcmPtr >> 2);
        const len = OpusEncoderLib._opus_encode_float(encoder, pcmPtr, FRAME_SIZE, opusPtr, MAX_PACKET);
        if (len > 0) {
            const out = new Uint8Array(OpusEncoderLib.HEAPU8.buffer, opusPtr, len);
            const buf = out.slice().buffer;
            self.postMessage({ type: 'opus', data: buf }, [buf]);
        }
        OpusEncoderLib._free(pcmPtr);
    } else if (msg.type === 'close') {
        if (encoder) OpusEncoderLib._opus_encoder_destroy(encoder);
        if (opusPtr) OpusEncoderLib._free(opusPtr);
        encoder = null;
        opusPtr = null;
        self.close();
    }
}

self.onmessage = (e) => {
    if (ready) handle(e.data);
    else pending.push(e.data);
};
