// PCM AudioWorklet：渲染量子（128 帧）仅在内部累积，出帧恒为
// 480 样本（10ms @48kHz，与引擎 hop 一致）——数据面不产生 1024/2048 等错位块。
// 帧经 postMessage 交主线程 → Opus WASM 编码器；输出恒静音（不本地回放麦克风）。
class PcmEncoderProcessor extends AudioWorkletProcessor {
    constructor() {
        super();
        this._buffer = new Float32Array(0);
        this._frameSize = 480;   // 10ms @ 48kHz（与引擎 hop 一致）
        this._frameCount = 0;
    }

    process(inputs, outputs) {
        const output = outputs[0][0];
        if (output) output.fill(0);
        const input = inputs[0][0];
        if (!input) return true;

        let peak = 0;
        for (let i = 0; i < input.length; i++) {
            const a = Math.abs(input[i]);
            if (a > peak) peak = a;
        }
        this.port.postMessage({ type: 'level', level: peak });

        const newBuf = new Float32Array(this._buffer.length + input.length);
        newBuf.set(this._buffer);
        newBuf.set(input, this._buffer.length);
        this._buffer = newBuf;

        while (this._buffer.length >= this._frameSize) {
            const frame = this._buffer.slice(0, this._frameSize);
            this._buffer = this._buffer.slice(this._frameSize);
            this._frameCount++;
            this.port.postMessage(
                { type: 'pcm', data: frame.buffer, n: this._frameCount,
                  backlog: this._buffer.length }, [frame.buffer]);
        }
        return true;
    }
}

registerProcessor('pcm-encoder-processor', PcmEncoderProcessor);
