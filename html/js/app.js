import { WsClient } from './ws-client.js';
import { AudioCapture } from './audio-capture.js';
import { Discovery } from './discovery.js';

class App {
    constructor() {
        this._wsClient = null;
        this._audioCapture = new AudioCapture();
        this._discovery = new Discovery();
        this._streaming = false;
        this._packetCount = 0;
        this._sendTimestamps = {};
        this._latencyAvg = 0;
        this._latencySamples = 0;
        this._deviceId = null;
        this._debugTimer = null;
        this._serverSampleRate = 0;
        this._bindUI();
        this._start();
    }

    _log(msg) {
        const el = document.getElementById('log-content');
        if (!el) return;
        const t = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
        el.innerHTML += `<div class="log-line">[${t}] ${msg}</div>`;
        el.scrollTop = el.scrollHeight;
        while (el.children.length > 100) el.removeChild(el.firstChild);
    }

    _bindUI() {
        this._elDiscovery = document.getElementById('discovery-status');
        this._elManual = document.getElementById('manual-connect');
        this._elServerInfo = document.getElementById('server-info');
        this._elServerName = document.getElementById('server-name');
        this._elStatus = document.getElementById('connection-status');
        this._elAudio = document.getElementById('audio-section');
        this._elMicBtn = document.getElementById('btn-mic');
        this._elMicText = document.getElementById('strip-text');
        this._elPeak = document.getElementById('vu-peak');
        this._elLatency = document.getElementById('latency');
        this._elPackets = document.getElementById('packet-count');
        this._elIp = document.getElementById('server-ip');
        this._elConnect = document.getElementById('btn-connect');
        this._elMicSelect = document.getElementById('mic-select');
        this._elError = document.getElementById('error-detail');
        this._elStripStreaming = document.getElementById('dbg-streaming');
        this._elMicBtn.addEventListener('click', () => this._toggleStream());
        this._elConnect.addEventListener('click', () => this._manualConnect());
        this._elMicSelect.addEventListener('change', () => {
            this._deviceId = this._elMicSelect.value || null;
            localStorage.setItem('purevox_mic_id', this._deviceId || '');
        });

        // ── 主题切换 ──
        this._themeToggle = document.getElementById('theme-toggle');
        this._applyTheme();
        this._themeToggle.addEventListener('click', () => {
            const root = document.documentElement;
            const current = root.className;
            if (!current) {
                root.className = 'theme-light';
            } else if (current === 'theme-light') {
                root.className = 'theme-dark';
            } else {
                root.className = ''; // 回到跟随系统
            }
            this._updateThemeIcon();
            localStorage.setItem('purevox_theme', root.className);
        });
        window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
            if (!document.documentElement.className) this._updateThemeIcon();
        });
    }

    _applyTheme() {
        const saved = localStorage.getItem('purevox_theme');
        if (saved !== null) document.documentElement.className = saved;
        this._updateThemeIcon();
    }

    _updateThemeIcon() {
        const root = document.documentElement;
        const cls = root.className;
        if (!cls) this._themeToggle.textContent = '◐';   // 跟随系统
        else if (cls === 'theme-light') this._themeToggle.textContent = '☀'; // 亮色
        else this._themeToggle.textContent = '☾';        // 深色
        this._themeToggle.title = !cls ? '跟随系统' : cls === 'theme-light' ? '亮色' : '深色';
    }

    async _start() {
        const curHost = window.location.hostname;
        const curPort = parseInt(window.location.port) || 59123;

        if (curHost && curHost !== 'localhost' && curHost !== '127.0.0.1') {
            this._elDiscovery.textContent = `正在连接 ${curHost}:${curPort}...`;
            const ok = await this._connectToServer(curHost, curPort);
            if (ok) return;
        }

        this._elDiscovery.textContent = '正在搜索局域网服务器...';
        this._discovery.onFound = (server) => this._onServerFound(server);
        const found = await this._discovery.discover();
        if (found.length === 0) {
            this._elDiscovery.textContent = '未找到服务器，请手动输入';
            this._elManual.style.display = 'flex';
        }
    }

    _onServerFound(server) {
        this._elDiscovery.style.display = 'none';
        this._elManual.style.display = 'none';
        this._elServerInfo.style.display = 'block';
        this._elServerName.textContent = `https://${server.ip}:${server.port}`;
        this._connectToServer(server.ip, server.port);
    }

    _manualConnect() {
        const val = this._elIp.value.trim();
        if (!val) return;
        const [ip, port] = val.includes(':') ? val.split(':') : [val, '59123'];
        this._elDiscovery.textContent = `正在连接 ${ip}:${port}...`;
        this._connectToServer(ip, parseInt(port));
    }

    async _enumerateMics() {
        try {
            const mics = await AudioCapture.listMics();
            const saved = localStorage.getItem('purevox_mic_id');
            const skipDefaultKeywords = ['虚拟', 'virtual', 'synthetic'];
            let firstRealId = null;
            let firstDefaultId = null;
            let html = '';
            mics.forEach((m, i) => {
                const label = m.label || `麦克风 ${i + 1}`;
                const lower = label.toLowerCase();
                html += `<option value="${m.deviceId}">${label}</option>`;
                if (lower.includes('默认') || lower.includes('default')) {
                    if (!firstDefaultId) firstDefaultId = m.deviceId;
                }
                if (!skipDefaultKeywords.some(k => lower.includes(k))) {
                    if (!firstRealId) firstRealId = m.deviceId;
                }
            });
            this._elMicSelect.innerHTML = html;
            const target = saved && mics.some(m => m.deviceId === saved) ? saved
                : firstDefaultId || firstRealId || (mics[0] && mics[0].deviceId);
            if (target) {
                this._elMicSelect.value = target;
                this._deviceId = target;
            }
        } catch (e) {
            console.warn('枚举麦克风失败:', e);
        }
    }

    async _fetchServerStatus() {
        try {
            const ip = this._wsClient && this._wsClient._url
                ? new URL(this._wsClient._url).hostname : window.location.hostname;
            const port = parseInt(window.location.port) || 59123;
            const resp = await fetch(`https://${ip}:${port}/api/status`);
            if (resp.ok) {
                const data = await resp.json();
                this._serverSampleRate = data.sample_rate || 0;
                this._log(`服务器状态: sample_rate=${data.sample_rate}, clients=${data.active_clients}`);
                return data;
            }
        } catch (e) {
            this._log(`获取服务器状态失败: ${e.message}`);
        }
        return null;
    }

    _updateDebugInfo() {
        const ac = this._audioCapture;
        setText('dbg-ctx-sr', ac.sampleRate ? ac.sampleRate + ' Hz' : '--');
        setText('dbg-device-sr', ac.deviceSampleRate ? ac.deviceSampleRate + ' Hz' : '--');
        setText('dbg-enc-sr', ac.encoderSampleRate + ' Hz');
        const frameMs = (ac.encoderFrameSize / 48000 * 1000).toFixed(0);
        setText('dbg-frame-size', ac.encoderFrameSize + ' (' + frameMs + 'ms)');
        setText('dbg-bitrate', (ac.encoderBitrate / 1000).toFixed(0) + 'k');
        setText('dbg-server-sr', this._serverSampleRate ? this._serverSampleRate + ' Hz' : '--');
        setText('dbg-backlog', ac.bufferBacklog.toLocaleString());
        setText('dbg-encoded', ac.totalEncoded.toLocaleString());

        // Streaming status
        if (this._elStripStreaming) {
            this._elStripStreaming.textContent = ac.streaming ? '● 推流' : '○ 停止';
            this._elStripStreaming.style.color = ac.streaming ? '#55cc66' : '#777';
        }

        // Highlight mismatch: ctx SR != encoder SR
        const ctxSr = ac.sampleRate;
        const dbgCtx = document.getElementById('dbg-ctx-sr');
        if (dbgCtx && ctxSr > 0 && ctxSr !== 48000) {
            dbgCtx.innerHTML = ctxSr + ' ⚠️≠48k';
        }

        // RTT & 估计总延迟
        const avgRtt = this._latencySamples > 0 ? this._latencyAvg : 0;
        setText('dbg-rtt', avgRtt > 0 ? avgRtt.toFixed(0) + 'ms' : '--');
        const frameMsNum = ac.encoderFrameSize / 48000 * 1000;
        const preFillMs = 3072 / 48000 * 1000;
        const targetAccMs = 5120 / 48000 * 1000;
        const estOneWay = avgRtt > 0 ? (avgRtt / 2) : 50;
        const totalLat = Math.round(estOneWay + frameMsNum + preFillMs + targetAccMs);
        const latColor = totalLat < 200 ? '#55cc66' : totalLat < 500 ? '#cccc44' : '#cc5544';
        setText('dbg-total-lat', `<span style="color:${latColor}">${totalLat}ms</span>`);

        function setText(id, html) {
            const el = document.getElementById(id);
            if (el) el.innerHTML = html;
        }
    }

    async _connectToServer(ip, port) {
        const wsUrl = `wss://${ip}:${port}/ws/audio`;
        this._wsClient = new WsClient(wsUrl);
        this._log(`正在连接服务器 ${ip}:${port}...`);

        this._wsClient.onOpen = () => {
            this._elStatus.textContent = '已连接';
            this._elStatus.className = 'sts connected';
            this._elAudio.style.display = 'block';
            this._elDiscovery.style.display = 'none';
            this._elManual.style.display = 'none';
            this._elServerInfo.style.display = 'block';
            this._elServerName.textContent = `https://${ip}:${port}`;
            this._enumerateMics();
            this._log('WebSocket 已连接');
            this._fetchServerStatus().then(() => this._updateDebugInfo());
        };

        this._wsClient.onClose = () => {
            this._elStatus.textContent = '未连接';
            this._elStatus.className = 'sts disconnected';
            this._streaming = false;
            this._elMicBtn.classList.remove('active');
            this._elMicText.textContent = '开始推流';
            if (this._debugTimer) {
                clearInterval(this._debugTimer);
                this._debugTimer = null;
            }
            this._log('WebSocket 已断开');
            this._updateDebugInfo();
        };

        this._wsClient.onAck = (seq) => {
            const sent = this._sendTimestamps[seq];
            if (sent) {
                delete this._sendTimestamps[seq];
                const rtt = Date.now() - sent;
                this._latencyAvg = (this._latencyAvg * this._latencySamples + rtt) / (this._latencySamples + 1);
                this._latencySamples = Math.min(this._latencySamples + 1, 100);
                this._elLatency.textContent = Math.round(this._latencyAvg);
            }
        };

        try {
            await this._wsClient.connect();
            return true;
        } catch (e) {
            const msg = e.message || '';
            const onHttps = location.protocol === 'https:';
            if (onHttps || msg.includes('cert') || msg.includes('SSL') || msg.includes('SECURITY')) {
                this._elDiscovery.innerHTML = `连接失败：SSL 证书问题<br>
                    <small>请先在浏览器中打开 <b>https://${ip}:${port}/</b> 并接受自签名证书，然后刷新本页</small>`;
            } else {
                this._elDiscovery.textContent = `连接失败: ${e.message || '未知错误'}`;
            }
            this._elManual.style.display = 'flex';
            return false;
        }
    }

    async _toggleStream() {
        if (this._streaming) {
            this._audioCapture.stop();
            this._wsClient.sendControl({ type: 'flush' });  // 清空服务端缓冲，不断开 WSS
            this._streaming = false;
            this._elMicBtn.classList.remove('active');
            this._elMicText.textContent = '开始推流';
            this._elMicBtn.querySelector('.strip-icon').textContent = '▶';
            this._elError.style.display = 'none';
            if (this._debugTimer) {
                clearInterval(this._debugTimer);
                this._debugTimer = null;
            }
            this._log('推流已停止');
            this._updateDebugInfo();
            return;
        }

        try {
            this._elError.style.display = 'none';
            this._log('正在启动麦克风...');
            this._audioCapture.onOpusData = (opusBytes) => {
                if (this._wsClient.connected) {
                    const now = Date.now();
                    const b64 = btoa(String.fromCharCode(...opusBytes));
                    const seq = this._wsClient.sendAudio(b64, now);
                    if (seq !== undefined) {
                        this._sendTimestamps[seq] = now;
                        this._packetCount++;
                        this._elPackets.textContent = this._packetCount;
                    }
                }
            };
            this._peakLevel = -60;
            this._peakTime = 0;
            this._audioCapture.onLevel = (level) => {
                const db = 20 * Math.log10(Math.max(level, 1e-6));
                const now = Date.now();
                if (db > this._peakLevel) { this._peakLevel = db; this._peakTime = now; }
                if (now - this._peakTime > 3000) {
                    this._peakLevel = Math.max(-60, this._peakLevel - 2);
                }
                const pct = Math.max(0, Math.min(99, ((db + 60) / 60) * 100));
                // 三段填充：各段独立宽度，绿→黄→红
                const f1 = document.getElementById('vu-fill1');
                const f2 = document.getElementById('vu-fill2');
                const f3 = document.getElementById('vu-fill3');
                if (f1) f1.style.width = Math.min(pct, 66.67) + '%';
                if (f2) f2.style.width = Math.max(0, Math.min(pct - 66.67, 18.33)) + '%';
                if (f3) f3.style.width = Math.max(0, Math.min(pct - 85, 15)) + '%';
                // 峰值（颜色跟随所在区域，不超过右边界）
                if (this._peakLevel > -59) {
                    const peakPct = Math.max(0, Math.min(99, ((this._peakLevel + 60) / 60) * 100));
                    this._elPeak.style.left = peakPct + '%';
                    const rootStyle = getComputedStyle(document.documentElement);
                    const peakColor = peakPct <= 66.67 ? rootStyle.getPropertyValue('--vu-fill1').trim()
                        : peakPct <= 85 ? rootStyle.getPropertyValue('--vu-fill2').trim()
                        : rootStyle.getPropertyValue('--vu-fill3').trim();
                    this._elPeak.style.background = peakColor || 'var(--vu-peak-color)';
                    this._elPeak.style.display = 'block';
                } else {
                    this._elPeak.style.display = 'none';
                }
            };

            await this._audioCapture.start(this._deviceId);
            this._streaming = true;
            this._elMicBtn.classList.add('active');
            this._elMicText.textContent = '停止推流';
            this._elMicBtn.querySelector('.strip-icon').textContent = '■';
            this._log(`麦克风已启动: ctx_sr=${this._audioCapture.sampleRate}, device_sr=${this._audioCapture.deviceSampleRate}`);
            this._updateDebugInfo();
            if (this._debugTimer) clearInterval(this._debugTimer);
            this._debugTimer = setInterval(() => this._updateDebugInfo(), 1000);
        } catch (e) {
            this._elError.textContent = '启动失败: ' + e.message;
            this._elError.style.display = 'block';
            this._log('启动失败: ' + e.message);
        }
    }
}

window.addEventListener('DOMContentLoaded', () => new App());
