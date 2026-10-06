export class Discovery {
    constructor() {
        this._onFound = null;
        this._servers = [];
    }

    async discover() {
        const curHost = window.location.hostname;
        const curPort = parseInt(window.location.port) || 59123;

        const candidates = [];

        if (curHost && curHost !== 'localhost' && curHost !== '127.0.0.1') {
            candidates.push({ ip: curHost, port: curPort });
        }

        const ipMatch = curHost.match(/^(\d+\.\d+\.\d+)\.\d+$/);
        if (ipMatch) {
            const base = ipMatch[1];
            for (let i = 1; i <= 254; i++) {
                const ip = `${base}.${i}`;
                if (ip === curHost) continue;
                candidates.push({ ip, port: curPort });
            }
        } else {
            const subnets = ['192.168.1', '192.168.0', '192.168.31', '10.0.0', '10.0.1'];
            for (const base of subnets) {
                for (let i = 1; i <= 254; i++) {
                    candidates.push({ ip: `${base}.${i}`, port: curPort });
                }
            }
        }

        const batchSize = 20;
        for (let i = 0; i < candidates.length; i += batchSize) {
            const batch = candidates.slice(i, i + batchSize);
            const results = await Promise.allSettled(batch.map(c => this._probe(c.ip, c.port)));
            for (const r of results) {
                if (r.status === 'fulfilled' && r.value) {
                    this._servers.push(r.value);
                    if (this._onFound) this._onFound(r.value);
                    return [r.value];
                }
            }
        }

        return [];
    }

    async _probe(ip, port) {
        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), 300);
        try {
            const resp = await fetch(`https://${ip}:${port}/api/status`, {
                signal: controller.signal,
            });
            clearTimeout(timer);
            if (resp.ok) {
                const data = await resp.json();
                return { ip, port, ...data };
            }
        } catch {
            clearTimeout(timer);
        }
        return null;
    }

    set onFound(fn) { this._onFound = fn; }
}
