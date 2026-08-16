// PureVox — AI 麦克风降噪工具（Qt C++ 重写版）
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
// GPL-3.0-or-later.  See LICENSE for details.
// The built-in AI models are NOT covered by the GPL (see MODEL-LICENSE.md).

#include "networkserver.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QWebSocket>
#include <QWebSocketServer>

#include <opus/opus.h>

#include <QThread>

#ifdef Q_OS_LINUX
#include "pwbackend.h"
#endif

#include <algorithm>
#include <cstring>

struct NetworkServer::OpusDecoderState {
    OpusDecoder *dec = nullptr;
    bool ok = false;
};

namespace {
// base64 → 原始字节
QByteArray fromBase64Str(const QByteArray &b64) {
    return QByteArray::fromBase64(b64);
}
}  // namespace

NetworkServer::NetworkServer(QObject *parent) : QObject(parent) {
    // Opus 解码器：48kHz 单声道
    opus_ = new OpusDecoderState;
    int err = 0;
    opus_->dec = opus_decoder_create(48000, 1, &err);
    opus_->ok = (err == OPUS_OK && opus_->dec != nullptr);
}

NetworkServer::~NetworkServer() {
    stop();
    if (opus_ && opus_->dec) opus_decoder_destroy(opus_->dec);
    delete opus_;
}

bool NetworkServer::start(int port, QString *err) {
    if (server_) return true;
    port_ = port;
    server_ = new QWebSocketServer(QStringLiteral("PureVox"), QWebSocketServer::NonSecureMode, this);
    if (!server_->listen(QHostAddress::Any, port_)) {
        QString e = QStringLiteral("监听失败: %1").arg(server_->errorString());
        if (err) *err = e;
        server_->deleteLater();
        server_ = nullptr;
        return false;
    }
    connect(server_, &QWebSocketServer::newConnection, this, &NetworkServer::onNewConnection);
    pcmBuf_.clear();
    playRunning_ = true;
    playThread_ = std::thread([this]() { playLoop(); });
    return true;
}

void NetworkServer::stop() {
    playRunning_ = false;
    if (playThread_.joinable()) playThread_.join();
    if (server_) {
        server_->close();
        server_->deleteLater();
        server_ = nullptr;
    }
    for (QWebSocket *c : clients_) {
        c->close();
        c->deleteLater();
    }
    clients_.clear();
    clientCount_ = 0;
}

int NetworkServer::activeClients() const { return clientCount_; }

void NetworkServer::onNewConnection() {
    QWebSocket *client = server_->nextPendingConnection();
    if (!client) return;
    clients_.push_back(client);
    clientCount_++;
    connect(client, &QWebSocket::textMessageReceived, this,
            [this, client](const QString &msg) { onTextMessage(client, msg); });
    connect(client, &QWebSocket::disconnected, this, [this, client]() {
        clients_.erase(std::remove(clients_.begin(), clients_.end(), client), clients_.end());
        clientCount_ = qMax(0, clientCount_ - 1);
        client->deleteLater();
    });
}

void NetworkServer::onTextMessage(QWebSocket *client, const QString &message) {
    QJsonParseError pe;
    QJsonDocument doc = QJsonDocument::fromJson(message.toUtf8(), &pe);
    if (pe.error != QJsonParseError::NoError || !doc.isObject()) return;
    QJsonObject obj = doc.object();
    QString type = obj.value("type").toString();
    if (type == QStringLiteral("audio")) {
        long seq = obj.value("seq").toVariant().toLongLong();
        QByteArray b64 = obj.value("data").toString().toUtf8();        QByteArray raw = fromBase64Str(b64);
        if (opus_->ok) {
            // Opus 解码 → 16-bit PCM → float
            int frame = 960;  // 20ms @48k
            std::vector<opus_int16> pcm(frame * 2);
            int n = opus_decode(opus_->dec, (const unsigned char *)raw.constData(),
                                raw.size(), pcm.data(), frame, 0);
            if (n > 0) {
                std::lock_guard<std::mutex> lock(bufMutex_);
                for (int i = 0; i < n; ++i) pcmBuf_.push_back(pcm[i] / 32768.0f);
                // 限制缓冲，防止无限增长
                if (pcmBuf_.size() > 48000 * 4) pcmBuf_.erase(pcmBuf_.begin(), pcmBuf_.begin() + 48000);
            }
        }
        // ACK
        QJsonObject ack;
        ack["type"] = "ack";
        ack["seq"] = QJsonValue((qint64)seq);
        if (client->isValid()) client->sendTextMessage(QString::fromUtf8(
            QJsonDocument(ack).toJson(QJsonDocument::Compact)));
    }
}

void NetworkServer::playLoop() {
#ifdef Q_OS_LINUX
    if (outputSink_.isEmpty()) return;
    // 仅输出流播放解码音频（input 留空 → 只建输出流）
    PwBackend backend;
    if (!backend.open(QString(), outputSink_, QString())) return;
    std::vector<float> chunk;
    while (playRunning_) {
        // 攒到 1024 样本写一次
        {
            std::lock_guard<std::mutex> lock(bufMutex_);
            if (pcmBuf_.size() >= 1024) {
                chunk.assign(pcmBuf_.begin(), pcmBuf_.begin() + 1024);
                pcmBuf_.erase(pcmBuf_.begin(), pcmBuf_.begin() + 1024);
            } else if (pcmBuf_.size() >= 256) {
                chunk = pcmBuf_;
                pcmBuf_.clear();
            } else {
                chunk.clear();
            }
        }
        if (!chunk.empty()) {
            backend.write(chunk.data(), chunk.size());
        } else {
            QThread::msleep(2);
        }
    }
    backend.close();
#else
    while (playRunning_) {
        std::lock_guard<std::mutex> lock(bufMutex_);
        pcmBuf_.clear();
        QThread::msleep(20);
    }
#endif
}

void NetworkServer::handleAudioPacket(QWebSocket *client, const QByteArray &opusBase64, long seq) {
    (void)client; (void)opusBase64; (void)seq;
}
