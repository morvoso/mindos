//! Try the Mind's web tools without a daemon, a model or a VM.
//!
//!     cargo run --example webtry -- search "nvidia 580 wayland"
//!     cargo run --example webtry -- fetch https://archlinux.org/news/
//!     cargo run --example webtry -- wiki "early kms"
//!     cargo run --example webtry -- wikipedia "RTX 4090"
//!     cargo run --example webtry -- proton "Elden Ring"
//!     cargo run --example webtry -- guard http://169.254.169.254/

use mindos_mind::config::WebConfig;
use mindos_mind::daemon::web;

fn head(v: serde_json::Value, field: &str, n: usize) -> serde_json::Value {
    let mut v = v;
    if let Some(t) = v[field].as_str() {
        v[field] = serde_json::json!(t.chars().take(n).collect::<String>());
    }
    v
}

#[tokio::main]
async fn main() {
    let cfg = WebConfig::default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    match arg(0).as_str() {
        "fetch" => {
            let p = web::fetch(&arg(1), &cfg, "text/html,application/json,*/*", cfg.max_bytes).await.unwrap();
            println!("status {} type {} bytes {} hops {:?}", p.status, p.content_type, p.body.len(), p.hops);
            let ex = web::html_to_text(&p.text(), url::Url::parse(&p.url).ok().as_ref());
            println!("TITLE: {}\n---\n{}\n---\nLINKS: {:?}", ex.title, ex.text.chars().take(1200).collect::<String>(), ex.links.iter().take(8).collect::<Vec<_>>());
        }
        "search" => println!("{:#}", web::search(&arg(1), 6, &cfg).await.unwrap()),
        "wiki" => println!("{:#}", head(web::wiki(&web::ARCH_WIKI, &arg(1), args.get(2).map(String::as_str), &cfg).await.unwrap(), "text", 800)),
        "wikipedia" => println!("{:#}", head(web::wiki(&web::WIKIPEDIA, &arg(1), args.get(2).map(String::as_str), &cfg).await.unwrap(), "text", 800)),
        "proton" => println!("{:#}", web::protondb(&arg(1), &cfg).await.unwrap()),
        "guard" => println!("{:?}", web::fetch(&arg(1), &cfg, "*/*", 1000).await.err()),
        other => eprintln!("unknown command {other:?}; see the comment at the top of this file"),
    }
}
