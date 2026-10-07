export class AudioCapture {
    constructor() {
        this._stream = null;
        this._context = null;
        this._processor = null;
        this._encoderWorker = null;
        this._onOpusData = null;
        this._onLevel = null;
        this._backlog = 0;
        this._frameSize = 480;   // 10ms @ 48kHz（与模型 hop 一致）
        // Debug info
        this._sampleRate = 0;
        this._deviceSampleRate = 0;
        this._totalEncoded = 0;
    }

    static async listMics() {
        const devices = await navigator.mediaDevices.enumerateDevices();
        return devices.filter(d => d.kind === 'audioinput');
    }

    async start(deviceId) {
        if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
            throw new Error('浏览器不支持麦克风访问（需要 HTTPS）');
        }

        this._context = new AudioContext();
        if (this._context.state === 'suspended') {
            await this._context.resume();
        }
        this._sampleRate = this._context.sampleRate;

        const constraints = { audio: { channelCount: 1 } };
        if (deviceId) {
            constraints.audio.deviceId = { exact: deviceId };
        }

        this._stream = await navigator.mediaDevices.getUserMedia(constraints);
        // Get actual device sample rate from MediaTrackSettings
        try {
            const track = this._stream.getAudioTracks()[0];
            const settings = track.getSettings();
            this._deviceSampleRate = settings.sampleRate || 0;
        } catch (e) {
            this._deviceSampleRate = 0;
        }
        const source = this._context.createMediaStreamSource(this._stream);

        this._encoderWorker = new Worker('wasm/opus-encoder-worker.js');
        this._encoderWorker.onmessage = (e) => {
            if (e.data.type === 'opus' && this._onOpusData) {
                this._onOpusData(new Uint8Array(e.data.data));
            }
        };

        // AudioWorklet：worklet 内按 480 样本（10ms）切帧后投递，
        // 数据面不出现 1024 等错位块；输出恒静音，连 destination
        // 仅为让节点保持被音频线程拉动。
        await this._context.audioWorklet.addModule('js/pcm-worklet.js');
        this._processor = new AudioWorkletNode(this._context, 'pcm-encoder-processor',
            { outputChannelCount: [1] });
        this._processor.port.onmessage = (e) => {
            const d = e.data;
            if (d.type === 'level') {
                if (this._onLevel) this._onLevel(d.level);
                return;
            }
            if (d.type === 'pcm') {
                this._totalEncoded++;
                this._backlog = d.backlog;
                if (this._onOpusData) {
                    this._encoderWorker.postMessage(
                        { type: 'encode', data: new Float32Array(d.data) },
                        [d.data]);
                }
            }
        };

        source.connect(this._processor);
        this._processor.connect(this._context.destination);
    }

    stop() {
        if (this._encoderWorker) {
            this._encoderWorker.postMessage({ type: 'close' });
            this._encoderWorker = null;
        }
        if (this._processor) {
            try { this._processor.disconnect(); } catch {}
            this._processor = null;
        }
        if (this._stream) {
            this._stream.getTracks().forEach(t => t.stop());
            this._stream = null;
        }
        if (this._context) {
            try { this._context.close(); } catch {}
            this._context = null;
        }
        this._backlog = 0;
        this._totalEncoded = 0;
    }

    set onOpusData(fn) { this._onOpusData = fn; }
    set onLevel(fn) { this._onLevel = fn; }

    // Debug info getters
    get sampleRate() { return this._sampleRate; }
    get deviceSampleRate() { return this._deviceSampleRate; }
    get encoderFrameSize() { return this._frameSize; }
    get encoderSampleRate() { return 48000; }     // Opus encoder hardcoded
    get encoderBitrate() { return 32000; }         // Opus encoder hardcoded
    get bufferBacklog() { return this._backlog; }
    get totalEncoded() { return this._totalEncoded; }
    get streaming() { return this._stream !== null; }
}
