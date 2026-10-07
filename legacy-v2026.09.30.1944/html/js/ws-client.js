export class WsClient {
    constructor(url) {
        this._url = url;
        this._ws = null;
        this._reconnectAttempts = 0;
        this._maxReconnects = 5;
        this._onOpen = null;
        this._onClose = null;
        this._onAck = null;
        this._seq = 0;
        this._intentionalClose = false;
    }

    connect() {
        return new Promise((resolve, reject) => {
            try {
                this._ws = new WebSocket(this._url);
            } catch (e) {
                reject(e);
                return;
            }
            this._ws.binaryType = 'arraybuffer';
            this._ws.onopen = () => {
                this._reconnectAttempts = 0;
                this._intentionalClose = false;
                if (this._onOpen) this._onOpen();
                resolve();
            };
            this._ws.onclose = () => {
                if (this._onClose) this._onClose();
                if (!this._intentionalClose) {
                    this._tryReconnect();
                }
            };
            this._ws.onerror = () => {
                reject(new Error(`WebSocket 连接失败（${this._url || ''}）`));
            };
            this._ws.onmessage = (e) => {
                try {
                    const msg = JSON.parse(e.data);
                    if (msg.type === 'ack' && this._onAck) {
                        this._onAck(msg.seq);
                    }
                } catch {}
            };
        });
    }

    sendAudio(frame, timestamp) {
        if (!this._ws || this._ws.readyState !== WebSocket.OPEN) return;
        const payload = JSON.stringify({
            type: 'audio',
            format: 'opus',
            data: frame,
            seq: this._seq,
            timestamp: timestamp || Date.now()
        });
        this._ws.send(payload);
        return this._seq++;
    }


    sendControl(msg) {
        if (!this._ws || this._ws.readyState !== WebSocket.OPEN) return;
        this._ws.send(JSON.stringify(msg));
    }

    disconnect() {
        this._intentionalClose = true;
        this._maxReconnects = 0;
        if (this._ws) {
            this._ws.close();
            this._ws = null;
        }
    }

    _tryReconnect() {
        if (this._reconnectAttempts >= this._maxReconnects) return;
        this._reconnectAttempts++;
        const delay = Math.min(1000 * Math.pow(2, this._reconnectAttempts), 10000);
        setTimeout(() => {
            const oldWs = this._ws;
            this._ws = null;
            if (oldWs) oldWs.close();
            this.connect().catch(() => {});
        }, delay);
    }

    set onOpen(fn) { this._onOpen = fn; }
    set onClose(fn) { this._onClose = fn; }
    set onAck(fn) { this._onAck = fn; }

    get connected() {
        return this._ws && this._ws.readyState === WebSocket.OPEN;
    }
}
