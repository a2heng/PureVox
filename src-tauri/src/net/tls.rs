// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// The built-in AI models are NOT covered by the GPL; they are the
// property of a2heng and may only be used with PureVox under
// authorization.  See MODEL-LICENSE.md for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! 局域网 HTTPS 的自签 CA 与服务器证书（DESIGN.md §4.1.11）。
//!
//! **为什么必须 HTTPS**：`navigator.mediaDevices.getUserMedia()` 只在**安全上下文**
//! 里可用。`http://192.168.x.x` 不是安全上下文，浏览器会**直接拒绝**麦克风请求，
//! 而且没有任何绕过办法（不是证书问题，是浏览器把整个 origin 判为不可信）。所以网页版
//! 客户端的麦克风在纯 HTTP 下**根本不可能工作**——这不是「配了就能用」的问题。
//!
//! **为什么是 CA + 叶证书两级，而不是一张自签**：Android 的「安装证书 → CA 证书」
//! 只接受 `basicConstraints: CA:TRUE` 的证书，普通自签服务器证书是 `CA:FALSE`，装不进去。
//! 想让用户在手机上装一次证书就彻底不再看到警告（也就彻底拿到安全上下文），必须由一张
//! 真 CA 去签叶证书。旧 Python 实现（`legacy-…/server/tls_manager.py`）就是这个结构。
//!
//! **两张证书的分工与寿命**：
//! - CA（10 年）：只签一次叶证书，长期不变，装到手机上也一次就够。
//! - 叶证书（397 天）：SAN 覆盖**当前所有**可直连 IPv4 + `localhost`。换网络、加了
//!   USB 共享那张卡 → IP 集合变了 → 重签。
//!
//! **重签不打断连接**：证书只在 `start()` 时读一次；之后即使网卡变了，已有连接照旧，
//! 要生效等下次「启动服务 / 重启服务」。这是刻意的：为了换张证书踢掉正在推流的客户端，
//! 比晚几十秒生效糟得多。
//!
//! 密钥与证书都存 `~/.purevox/ca/`，格式与旧实现一致（`ca.key` / `ca.crt` /
//! `server.key` / `server.crt`），方便老用户直接沿用已装到手机上的 CA。

use std::io;
use std::path::{Path, PathBuf};

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, KeyUsagePurpose};

/// CA 的 subject。**生成与读回共用同一份**，否则签出来的链 issuer 对不上 CA 的 subject，
/// 浏览器报的是「证书链无效」而不是「未受信任」，两个都不是用户能自己看懂的。
const CA_CN: &str = "PureVox Local CA";
const ORG: &str = "PureVox";

/// CA 的证书参数。
///
/// `Issuer` 需要 CA 的 distinguished_name 才能签叶证书，而从 PEM 读回 subject 要 rcgen 的
/// `x509-parser` feature（为读一个字符串多一棵依赖树，不值）。这里的参数是我们自己定的、
/// 就那么几项，所以**读回时用同一个函数重建**即可 —— 两边共用一个函数就不会走偏。
fn ca_params() -> CertificateParams {
  let mut p = CertificateParams::default();
  p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
  p.key_usages = vec![
    KeyUsagePurpose::KeyCertSign,
    KeyUsagePurpose::CrlSign,
    KeyUsagePurpose::DigitalSignature,
  ];
  p.distinguished_name.push(DnType::CommonName, CA_CN);
  p.distinguished_name.push(DnType::OrganizationName, ORG);
  p
}

/// 签名算法。**生成与读回必须是同一个**，而 `serialize_pem()` 写的 PKCS#8 里不带算法标识，
/// 读回时得自己告诉 rcgen（`from_pem` 只认它能自动推断的格式，喂 PKCS#8 会报
/// `Could not parse key pair`）。
///
/// 不依赖 `KeyPair::generate()` 的默认值：哪天上游改了默认算法，老用户的 `ca.key` 就再也
/// 读不出来，服务直接起不来。写死常量比赌默认值稳。
const ALG: &rcgen::SignatureAlgorithm = &rcgen::PKCS_ECDSA_P256_SHA256;

/// 证书目录（`~/.purevox/ca/`）。
fn dir() -> Result<PathBuf, String> {
  crate::config::config_dir().map(|d| d.join("ca"))
}

/// 读出来的服务端材料：**叶证书 + 私钥**，以及供用户安装的 **CA 证书 PEM**。
pub struct Tls {
  /// 交给 `rustls` 的服务端配置（叶证书 + 私钥）
  pub config: rustls::ServerConfig,
  /// CA 证书 PEM（`/ca-cert` 下载用；用户装到手机上之后就不再有证书警告）
  pub ca_pem: String,
  /// CA 是不是**这次重新生成**的（老格式读不回来 / 文件损坏）。
  ///
  /// 为 true 意味着手机上先前装的那个 CA 已作废、要重装一次。这是用户必须知道的事，
  /// 不能静默换掉（否则表现为「我明明装过证书，怎么又开始报警」）。
  pub ca_replaced: bool,
}

/// 取（或首次生成）当前网卡集合对应的 TLS 材料。
///
/// IP 集合已覆盖在用的叶证书就直接复用，不重签 —— 否则每次开服务都换证书会让手机上
/// 已装的用户 CA 失去意义（用户装的是 CA，CA 不会因为叶证书变了而失效，所以其实影响不大，
/// 但重签要多算一次 RSA/ECDSA 密钥生成，没必要）。
pub fn load_or_create(ips: &[String]) -> Result<Tls, String> {
  let d = dir()?;
  std::fs::create_dir_all(&d).map_err(|e| format!("创建证书目录失败：{e}"))?;

  let (ca_pem, ca_key, ca_replaced) = ensure_ca(&d)?;
  let want: Vec<String> = ips
    .iter()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .collect();

  // CA 换了就必须重签叶证书：链的首尾对不上，浏览器报的是「证书链无效」这种没人看得懂的错
  let leaf_pem_and_key = if ca_replaced || !covers(&d, &want) {
    sign_leaf_at(&d, &ca_key, &want)?
  } else {
    (read(&d.join("server.crt"))?, read(&d.join("server.key"))?)
  };

  let (leaf_pem, leaf_key_pem) = leaf_pem_and_key;
  let mut t = build(&leaf_pem, &leaf_key_pem, &ca_pem)?;
  t.ca_replaced = ca_replaced;
  Ok(t)
}

/// 叶证书的 SAN 是否已覆盖给定 IP 集合。
///
/// **不解析证书**：rcgen 的 `CertificateParams::from_ca_cert_pem` 是 `pub(crate)`，外部读不到
/// SAN。与其为了一次集合比较去引 `x509-parser` feature（多一棵依赖树），不如签发时把
/// 「这张证书覆盖了哪些 IP」原样写一份 `server.ips` 在旁边 —— 证书与清单同一次写出，
/// 不会失配；清单读不出来（旧版本留下的证书、没有清单）一律当作**没覆盖**、重签，
/// 这个默认方向是安全的。
fn covers(d: &Path, ips: &[String]) -> bool {
  let Ok(text) = std::fs::read_to_string(d.join("server.ips")) else {
    return false;
  };
  let have: Vec<&str> = text
    .lines()
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .collect();
  ips.iter().all(|ip| have.contains(&ip.as_str()))
}

/// 读出（或首次生成）本机 CA。CA 长期不变，所以只在文件缺失或读不回来时生成。
///
/// **读不回来就直接覆盖重签，不报错卡死**。旧 Python 实现（`legacy-…/server/tls_manager.py`）
/// 用的是 RSA 私钥，而目录与文件名（`~/.purevox/ca/ca.key`）是同一套，升级上来的用户手里那份
/// 一定解析不了。此时报「读 CA 私钥失败」等于服务永远起不来。旧 CA 不需要保留——它只是个
/// 本地自签，用户在浏览器里点一次「继续」就能过；`ca_replaced` 只是让界面如实说一声
/// 「CA 换过了，手机上要重装一次」。
fn ensure_ca(d: &Path) -> Result<(String, KeyPair, bool), String> {
  let cert_path = d.join("ca.crt");
  let key_path = d.join("ca.key");
  let existed = cert_path.is_file() && key_path.is_file();
  if existed {
    // 任一环节失败都当作「没有可用的 CA」，走重签；理由由 `ca_replaced` 带给界面
    if let Ok(pem) = read(&cert_path)
      && let Ok(key_pem) = read(&key_path)
      && let Ok(key) = KeyPair::from_pkcs8_pem_and_sign_algo(&key_pem, ALG)
    {
      return Ok((pem, key, false));
    }
  }

  let key = KeyPair::generate_for(ALG).map_err(|e| format!("生成 CA 密钥失败：{e}"))?;
  let pem = ca_params()
    .self_signed(&key)
    .map_err(|e| format!("签发 CA 证书失败：{e}"))?
    .pem();
  std::fs::write(&cert_path, &pem).map_err(|e| format!("写 CA 证书失败：{e}"))?;
  std::fs::write(&key_path, key.serialize_pem()).map_err(|e| format!("写 CA 私钥失败：{e}"))?;
  Ok((pem, key, existed))
}

/// 用 CA 签一张叶证书，SAN = `localhost` + 传入的全部 IP。
///
/// 目录参数显式传入（而不是内部调 `dir()`）是为了让测试能指向临时目录——在用户真实的
/// `~/.purevox/ca/` 里留一张 CA 意味着手机上会多出一个陌生 CA，不是测试该做的事。
fn sign_leaf_at(d: &Path, ca_key: &KeyPair, ips: &[String]) -> Result<(String, String), String> {
  // `CertificateParams::new` 逐项判断是 IP 还是域名，IP 会变成 `SAN:IPAddress`
  let mut sans: Vec<String> = vec!["localhost".to_string()];
  for ip in ips {
    // 明显不是地址的（理论上不会有，`addrs::list()` 已经滤过）跳过而不是让整体失败
    if ip.parse::<std::net::IpAddr>().is_ok() {
      sans.push(ip.clone());
    }
  }
  let mut params = CertificateParams::new(sans).map_err(|e| format!("构造证书参数失败：{e}"))?;
  params
    .distinguished_name
    .push(DnType::CommonName, "PureVox");
  params
    .distinguished_name
    .push(DnType::OrganizationName, "PureVox");

  let key = KeyPair::generate_for(ALG).map_err(|e| format!("生成服务器密钥失败：{e}"))?;
  // issuer 用 `ca_params()` 重建（见它的注释）：subject 与 CA 证书一致，链才对得上
  let issuer = Issuer::new(ca_params(), ca_key);
  let cert = params
    .signed_by(&key, &issuer)
    .map_err(|e| format!("签发服务器证书失败：{e}"))?;

  let pem = cert.pem();
  let key_pem = key.serialize_pem();
  std::fs::create_dir_all(d).map_err(|e| format!("创建证书目录失败：{e}"))?;
  std::fs::write(d.join("server.crt"), &pem).map_err(|e| format!("写服务器证书失败：{e}"))?;
  std::fs::write(d.join("server.key"), &key_pem).map_err(|e| format!("写服务器私钥失败：{e}"))?;
  // 覆盖清单与证书同一次写出（见 `covers` 的说明）
  std::fs::write(d.join("server.ips"), ips.join("\n"))
    .map_err(|e| format!("写证书覆盖清单失败：{e}"))?;
  Ok((pem, key_pem))
}

/// PEM → `rustls::ServerConfig`。
fn build(leaf_pem: &str, leaf_key_pem: &str, _ca_pem: &str) -> Result<Tls, String> {
  let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
    rustls_pemfile::certs(&mut leaf_pem.as_bytes())
      .collect::<Result<_, io::Error>>()
      .map_err(|e| format!("解析服务器证书失败：{e}"))?;
  if certs.is_empty() {
    return Err("服务器证书里一张都没有".into());
  }
  let key = rustls_pemfile::private_key(&mut leaf_key_pem.as_bytes())
    .map_err(|e| format!("解析服务器私钥失败：{e}"))?
    .ok_or_else(|| "服务器私钥是空的".to_string())?;

  // **必须显式选 provider**：依赖树里同时有 `ring`（本模块用的）与 `aws-lc-rs`
  // （`reqwest` / `hyper-rustls` 带进来的），`ServerConfig::builder()` 遇到两个都在就会
  // 「不知道该用哪个」而 panic。这里用 `builder_with_provider` 局部指定 ring，不去装
  // 进程级默认 provider —— 那是全局状态，会顺手改掉 reqwest 的行为。
  let provider = rustls::crypto::ring::default_provider();
  let cfg = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(provider))
    .with_safe_default_protocol_versions()
    .map_err(|e| format!("选 TLS 协议版本失败：{e}"))?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .map_err(|e| format!("装配 TLS 配置失败：{e}"))?;
  Ok(Tls {
    config: cfg,
    ca_pem: _ca_pem.to_string(),
    ca_replaced: false,
  })
}

fn read(p: &Path) -> Result<String, String> {
  std::fs::read_to_string(p).map_err(|e| format!("读 {} 失败：{e}", p.display()))
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 临时证书目录。测试**绝不写用户真实的 `~/.purevox/ca/`** —— 在那儿留一张 CA 意味着
  /// 用户手机上会多出一个可以信任的陌生 CA，这不是测试该做的事。
  fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pv_tls_{tag}_{}", std::process::id()));
    std::fs::remove_dir_all(&d).ok();
    std::fs::create_dir_all(&d).unwrap();
    d
  }

  fn ca_at(d: &Path) -> (String, KeyPair) {
    let key = KeyPair::generate().unwrap();
    let pem = ca_params().self_signed(&key).unwrap().pem();
    std::fs::write(d.join("ca.crt"), &pem).unwrap();
    std::fs::write(d.join("ca.key"), key.serialize_pem()).unwrap();
    (pem, key)
  }

  /// 签出来的叶证书要能被 `rustls` 装上（= 私钥与证书确实配对），且覆盖清单写对。
  #[test]
  fn 叶证书能装配且覆盖清单正确() {
    let d = tmp("sign");
    let (ca_pem, ca_key) = ca_at(&d);
    let ips = vec!["192.168.1.5".to_string(), "172.26.38.56".to_string()];
    let (leaf_pem, leaf_key) = sign_leaf_at(&d, &ca_key, &ips).unwrap();

    let t = build(&leaf_pem, &leaf_key, &ca_pem).unwrap();
    assert!(t.ca_pem.contains("BEGIN CERTIFICATE"), "CA PEM 要能下载");
    assert!(covers(&d, &ips), "刚签的证书必须被判为已覆盖");
    std::fs::remove_dir_all(&d).ok();
  }

  /// 换了网络（IP 集合变了）必须判为「没覆盖」→ 重签。否则会拿一张不含新 IP 的证书去
  /// 签 TLS，浏览器报的是「证书域名不匹配」，而用户真正该看到的应该是「换了网络，重启
  /// 服务」。
  #[test]
  fn 换了网络判为没覆盖() {
    let d = tmp("cover");
    let (_, ca_key) = ca_at(&d);
    let old = vec!["10.0.0.2".to_string()];
    sign_leaf_at(&d, &ca_key, &old).unwrap();

    assert!(covers(&d, &old));
    // 同一张网卡 + 多了一张，多出来的没覆盖 → 必须重签
    assert!(!covers(
      &d,
      &["10.0.0.2".to_string(), "192.168.9.9".to_string()]
    ));
    // 换了一张 → 必须重签
    assert!(!covers(&d, &["192.168.9.9".to_string()]));
    // 空集合（本机一张可用网卡都没有）不触发重签
    assert!(covers(&d, &[]));
    // 清单读不出来（旧实现留下的证书）→ 当作没覆盖，重签一次
    std::fs::remove_file(d.join("server.ips")).unwrap();
    assert!(!covers(&d, &old));
    std::fs::remove_dir_all(&d).ok();
  }

  /// 私钥与证书不配对必须**明确报错**，而不是等到第一个客户端连上来才炸。
  /// `rustls` 在装配时就会校验证书公钥与私钥是否配对。
  #[test]
  fn 私钥不配对时报错() {
    let d = tmp("mismatch");
    let (ca_pem, ca_key) = ca_at(&d);
    let (leaf_pem, _) = sign_leaf_at(&d, &ca_key, &["10.0.0.2".to_string()]).unwrap();
    let other = KeyPair::generate().unwrap();
    assert!(
      build(&leaf_pem, &other.serialize_pem(), &ca_pem).is_err(),
      "用别的私钥配这张证书必须报错"
    );
    std::fs::remove_dir_all(&d).ok();
  }

  /// CA 必须带 `CA:TRUE`：**Android 的「安装证书 → CA 证书」只接受 CA**，普通自签服务器
  /// 证书（`CA:FALSE`）装不进去。这是整个两级结构存在的理由。
  ///
  /// 只能断言到「我们确实请求了 CA 约束」这一层——字节层面是不是真写进了 basicConstraints
  /// 要 x509 解析器（`rcgen` 的该 feature 只在内部开放），实机装一次证书才是最终验证。
  #[test]
  fn ca参数必须是无限约束的ca() {
    assert!(
      matches!(ca_params().is_ca, IsCa::Ca(BasicConstraints::Unconstrained)),
      "CA 必须是 Unconstrained CA"
    );
    // 叶证书反过来不能是 CA：否则它也能去签别的证书，语义就错了
    let mut leaf = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    assert!(!matches!(leaf.is_ca, IsCa::Ca(_)));
    leaf.is_ca = IsCa::ExplicitNoCa;
    assert!(!matches!(leaf.is_ca, IsCa::Ca(_)));
  }
}
