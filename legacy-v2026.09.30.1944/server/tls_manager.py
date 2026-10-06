# PureVox — AI 麦克风降噪工具
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
#
# PureVox is licensed under the GNU General Public License v3.0 or
# later (GPL-3.0-or-later).  See LICENSE for details.
# 
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
# 
# The built-in AI models are NOT covered by the GPL; they are the
# property of a2heng and may only be used with PureVox under
# authorization.  See MODEL-LICENSE.md for details.
# 
# SPDX-License-Identifier: GPL-3.0-or-later

import ipaddress
import os
import ssl
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import List

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID


class TlsManager:
    def __init__(self, cert_dir: str = ""):
        if not cert_dir:
            cert_dir = os.path.join(os.path.expanduser("~"), ".purevox", "ca")
        self._cert_dir = Path(cert_dir)
        self._cert_dir.mkdir(parents=True, exist_ok=True)
        self._ca_key_path = self._cert_dir / "ca.key"
        self._ca_cert_path = self._cert_dir / "ca.crt"
        self._server_key_path = self._cert_dir / "server.key"
        self._server_cert_path = self._cert_dir / "server.crt"
        self._ca_cert = None
        self._ca_key = None

    def ensure_ca(self):
        if self._ca_key_path.exists() and self._ca_cert_path.exists():
            self._ca_key = serialization.load_pem_private_key(
                self._ca_key_path.read_bytes(), password=None)
            self._ca_cert = x509.load_pem_x509_certificate(
                self._ca_cert_path.read_bytes())
            return
        self._ca_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        subject = issuer = x509.Name([
            x509.NameAttribute(NameOID.COMMON_NAME, "PureVox Local CA"),
            x509.NameAttribute(NameOID.ORGANIZATION_NAME, "PureVox"),
        ])
        self._ca_cert = (
            x509.CertificateBuilder()
            .subject_name(subject)
            .issuer_name(issuer)
            .public_key(self._ca_key.public_key())
            .serial_number(x509.random_serial_number())
            .not_valid_before(datetime.now(timezone.utc))
            .not_valid_after(datetime.now(timezone.utc) + timedelta(days=3650))
            .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
            .sign(self._ca_key, hashes.SHA256())
        )
        self._ca_key_path.write_bytes(self._ca_key.private_bytes(
            serialization.Encoding.PEM, serialization.PrivateFormat.TraditionalOpenSSL,
            serialization.NoEncryption()))
        self._ca_cert_path.write_bytes(self._ca_cert.public_bytes(serialization.Encoding.PEM))

    def server_cert_covers(self, ip_addresses: List[str]) -> bool:
        """现有 server 证书的 SAN 是否已覆盖给定 IP（无证书/读失败返回 False）。"""
        if not self._server_cert_path.exists():
            return False
        try:
            cert = x509.load_pem_x509_certificate(self._server_cert_path.read_bytes())
            san = cert.extensions.get_extension_for_class(
                x509.SubjectAlternativeName).value
            # 显式遍历（部分 cryptography 版本 get_values_for_type 对 IPv4 返回空）
            have = {str(g.value) for g in san if isinstance(g, x509.IPAddress)}
        except Exception:
            return False
        return {str(ip) for ip in ip_addresses}.issubset(have)

    def reload_ssl_context(self, ctx: ssl.SSLContext) -> None:
        """把最新证书链热加载进已有 SSLContext（不中断现有连接）。"""
        ctx.load_cert_chain(str(self._server_cert_path), str(self._server_key_path))

    def generate_server_cert(self, ip_addresses: List[str], force: bool = False):
        if self._ca_key is None or self._ca_cert is None:
            raise RuntimeError("Call ensure_ca() before generate_server_cert()")
        if not force and self._server_key_path.exists() and self._server_cert_path.exists():
            return
        server_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        subject = x509.Name([
            x509.NameAttribute(NameOID.COMMON_NAME, "PureVox Server"),
        ])
        san_list = [x509.DNSName("localhost")]
        for ip_str in ip_addresses:
            san_list.append(x509.IPAddress(ipaddress.ip_address(ip_str)))
        cert = (
            x509.CertificateBuilder()
            .subject_name(subject)
            .issuer_name(self._ca_cert.subject)
            .public_key(server_key.public_key())
            .serial_number(x509.random_serial_number())
            .not_valid_before(datetime.now(timezone.utc))
            .not_valid_after(datetime.now(timezone.utc) + timedelta(days=365))
            .add_extension(x509.SubjectAlternativeName(san_list), critical=False)
            .sign(self._ca_key, hashes.SHA256())
        )
        self._server_key_path.write_bytes(server_key.private_bytes(
            serialization.Encoding.PEM, serialization.PrivateFormat.TraditionalOpenSSL,
            serialization.NoEncryption()))
        self._server_cert_path.write_bytes(cert.public_bytes(serialization.Encoding.PEM))

    def get_ssl_context(self) -> ssl.SSLContext:
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(str(self._server_cert_path), str(self._server_key_path))
        return ctx

    def get_ca_cert_pem(self) -> bytes:
        return self._ca_cert_path.read_bytes()
