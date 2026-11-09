//! Listener-aware local and LAN URLs. Address discovery never sends network traffic.
use serde_json::{Value, json};
use std::net::IpAddr;
fn url(ip: IpAddr, port: u16) -> String {
    format!(
        "http://{}:{port}",
        match ip {
            IpAddr::V4(v) => v.to_string(),
            IpAddr::V6(v) => format!("[{v}]"),
        }
    )
}
fn addresses(bind: IpAddr, port: u16, interfaces: Vec<(String, IpAddr)>) -> Value {
    let mut lan=interfaces.into_iter().filter(|(_,ip)|{
        if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() || ip.is_ipv4()!=bind.is_ipv4(){return false;}
        match ip {IpAddr::V4(v)=>!v.is_link_local(),IpAddr::V6(v)=>(v.segments()[0]&0xffc0)!=0xfe80}
    }).map(|(name,ip)|json!({"interface":name,"url":url(ip,port),"accessible":bind.is_unspecified() || bind==ip})).collect::<Vec<_>>();
    lan.sort_by_key(|v| v["url"].as_str().unwrap().to_owned());
    lan.dedup_by(|a, b| a["url"] == b["url"]);
    let local_accessible = bind.is_unspecified() || bind.is_loopback();
    json!({"listen_address":std::net::SocketAddr::new(bind,port).to_string(),"local_url":format!("http://localhost:{port}"),"local_accessible":local_accessible,"lan_addresses":lan,"lan_hint":if bind.is_loopback(){Some("Loopback only. Restart with --host to allow IPv4 LAN clients.")}else{None}})
}
pub fn discover(host: &str, port: u16) -> Value {
    let bind = host.parse().expect("validated bind IP");
    let interfaces = if_addrs::get_if_addrs()
        .map(|list| {
            list.into_iter()
                .map(|entry| {
                    let ip = entry.ip();
                    (entry.name, ip)
                })
                .collect()
        })
        .unwrap_or_default();
    addresses(bind, port, interfaces)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loopback_reports_lan_as_inaccessible_and_wildcard_enables_it() {
        let interfaces = vec![
            ("en0".into(), "192.168.1.50".parse().unwrap()),
            ("lo0".into(), "127.0.0.1".parse().unwrap()),
            ("unused".into(), "169.254.1.2".parse().unwrap()),
        ];
        let local = addresses("127.0.0.1".parse().unwrap(), 9966, interfaces.clone());
        assert_eq!(local["local_url"], "http://localhost:9966");
        assert_eq!(local["lan_addresses"].as_array().unwrap().len(), 1);
        assert_eq!(local["lan_addresses"][0]["accessible"], false);
        assert!(local["lan_hint"].is_string());
        let lan = addresses("0.0.0.0".parse().unwrap(), 9966, interfaces.clone());
        assert_eq!(lan["lan_addresses"][0]["accessible"], true);
        let bound = addresses("192.168.1.50".parse().unwrap(), 9966, interfaces);
        assert_eq!(bound["local_accessible"], false);
        assert_eq!(bound["lan_addresses"][0]["accessible"], true);
    }
    #[test]
    fn ipv6_urls_are_bracketed_and_link_local_is_omitted() {
        let data = addresses(
            "::".parse().unwrap(),
            9966,
            vec![
                ("en0".into(), "fd00::123".parse().unwrap()),
                ("en1".into(), "fe80::1".parse().unwrap()),
            ],
        );
        assert_eq!(data["lan_addresses"].as_array().unwrap().len(), 1);
        assert_eq!(data["lan_addresses"][0]["url"], "http://[fd00::123]:9966");
    }
}
