// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#ifndef PUREVOX_NETWORKSERVER_H
#define PUREVOX_NETWORKSERVER_H

#include <QObject>
#include <QString>

#include <memory>
#include <mutex>
#include <thread>
#include <vector>

class QWebSocketServer;
class QWebSocket;
class AudioBackend;
struct OpusDecoder;

// 远程麦克风推流服务器：接收浏览器/Android 的 Opus 音频，解码后写本地扬声器
class NetworkServer : public QObject {
    Q_OBJECT

public:
    explicit NetworkServer(QObject *parent = nullptr);
    ~NetworkServer() override;

    bool start(int port, QString *err);
    void stop();
    bool running() const { return server_ != nullptr; }
    int port() const { return port_; }
    int activeClients() const;

    // 本地播放输出节点名（Linux PipeWire sink）
    void setOutputSink(const QString &sink) { outputSink_ = sink; }

private:
    void onNewConnection();
    void onTextMessage(QWebSocket *client, const QString &message);
    void handleAudioPacket(QWebSocket *client, const QByteArray &opusBase64, long seq);
    void playLoop();

    QWebSocketServer *server_ = nullptr;
    int port_ = 8443;
    QString outputSink_;

    struct OpusDecoderState;
    OpusDecoderState *opus_ = nullptr;

    // 接收缓冲（PCM float, 48kHz 单声道）
    std::vector<float> pcmBuf_;
    std::mutex bufMutex_;
    bool playRunning_ = false;
    std::thread playThread_;
    std::vector<QWebSocket *> clients_;
    int clientCount_ = 0;
};

#endif // PUREVOX_NETWORKSERVER_H
