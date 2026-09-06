//! `window.mindos`: the bootstrap script injected into every view and the
//! JavaScript snippets the host evaluates to answer calls and push events.

use serde::Deserialize;
use serde_json::Value;

/// Injected at document start into every frame, before any page script.
pub const BOOTSTRAP: &str = r#"(function () {
  if (window.mindos && window.mindos._host) return;
  var pending = new Map();
  var listeners = new Map();
  var seq = 0;
  var params = new URLSearchParams(location.search);
  var info = { kind: params.get('kind') || 'preview', id: params.get('id') || '', output: params.get('output') || '' };
  if (params.has('popup')) info.popup = params.get('popup');
  if (params.has('arg')) { try { info.arg = JSON.parse(params.get('arg')); } catch (e) { info.arg = params.get('arg'); } }
  if (params.has('anchor')) { try { info.anchor = JSON.parse(params.get('anchor')); } catch (e) {} }
  var mindos = {
    _host: true,
    window: info,
    call: function (method, p) {
      return new Promise(function (resolve, reject) {
        var id = ++seq;
        pending.set(id, { resolve: resolve, reject: reject });
        try {
          window.webkit.messageHandlers.mindos.postMessage(JSON.stringify({ id: id, method: method, params: p === undefined ? null : p }));
        } catch (e) { pending.delete(id); reject(e); }
      });
    },
    on: function (event, cb) {
      var set = listeners.get(event);
      if (!set) { set = new Set(); listeners.set(event, set); }
      set.add(cb);
      return function () { set.delete(cb); };
    },
    _reply: function (id, ok, payload) {
      var p = pending.get(id);
      if (!p) return;
      pending.delete(id);
      if (ok) p.resolve(payload);
      else p.reject(new Error(typeof payload === 'string' ? payload : JSON.stringify(payload)));
    },
    _dispatch: function (event, payload) {
      var set = listeners.get(event);
      if (set) Array.from(set).forEach(function (cb) { try { cb(payload); } catch (e) { console.error('mindos.on(' + event + ')', e); } });
      var all = listeners.get('*');
      if (all) Array.from(all).forEach(function (cb) { try { cb({ event: event, payload: payload }); } catch (e) {} });
    }
  };
  Object.defineProperty(window, 'mindos', { value: mindos, writable: true, configurable: true, enumerable: true });
})();"#;

/// A request from a view.
#[derive(Debug, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

pub fn parse_request(text: &str) -> Result<Request, String> {
    let req: Request = serde_json::from_str(text).map_err(|e| format!("bad bridge message: {e}"))?;
    if req.method.is_empty() {
        return Err("bridge message without method".into());
    }
    Ok(req)
}

/// A JSON value as a JavaScript expression (JSON is a JavaScript subset, but
/// `</script>` and the U+2028/2029 line separators must not break the source).
pub fn js_literal(value: &Value) -> String {
    let text = value.to_string();
    text.replace("</", "<\\/").replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}

pub fn reply_js(id: u64, ok: bool, payload: &Value) -> String {
    format!("window.mindos && window.mindos._reply({id}, {ok}, {});", js_literal(payload))
}

pub fn dispatch_js(event: &str, payload: &Value) -> String {
    format!(
        "window.mindos && window.mindos._dispatch({}, {});",
        js_literal(&Value::String(event.to_string())),
        js_literal(payload)
    )
}

/// Build the URL a window loads.
pub fn window_url(kind: &str, id: &str, output: &str, popup: Option<&str>, arg: Option<&Value>, anchor: Option<&Value>) -> String {
    use crate::icons::percent_encode as enc;
    let mut url = format!(
        "{}/app/index.html?kind={}&id={}&output={}",
        crate::scheme::ORIGIN,
        enc(kind),
        enc(id),
        enc(output)
    );
    if let Some(p) = popup {
        url.push_str("&popup=");
        url.push_str(&enc(p));
    }
    if let Some(a) = arg.filter(|a| !a.is_null()) {
        url.push_str("&arg=");
        url.push_str(&enc(&a.to_string()));
    }
    if let Some(a) = anchor.filter(|a| !a.is_null()) {
        url.push_str("&anchor=");
        url.push_str(&enc(&a.to_string()));
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_requests() {
        let r = parse_request(r#"{"id":3,"method":"apps.launch","params":{"id":"steam.desktop"}}"#).unwrap();
        assert_eq!(r.id, Some(3));
        assert_eq!(r.method, "apps.launch");
        assert_eq!(r.params["id"], "steam.desktop");
        assert!(parse_request("{}").is_err());
        assert!(parse_request("not json").is_err());
        assert!(parse_request(r#"{"method":"x","params":null}"#).is_ok());
    }

    #[test]
    fn js_is_safe() {
        let js = reply_js(1, true, &json!({"a": "</script>\u{2028}"}));
        assert!(!js.contains("</script>"));
        assert!(!js.contains('\u{2028}'));
        assert!(js.starts_with("window.mindos && window.mindos._reply(1, true, "));
        let d = dispatch_js("windows", &json!({"windows": [], "focused": null}));
        assert_eq!(d, r#"window.mindos && window.mindos._dispatch("windows", {"focused":null,"windows":[]});"#);
    }

    #[test]
    fn builds_urls() {
        let u = window_url("popup", "launcher", "DP-1", Some("launcher"), Some(&json!({"q": "a b"})), None);
        assert_eq!(
            u,
            "mindos://shell/app/index.html?kind=popup&id=launcher&output=DP-1&popup=launcher&arg=%7B%22q%22%3A%22a%20b%22%7D"
        );
        assert_eq!(window_url("desktop", "desktop", "DP-1", None, None, None), "mindos://shell/app/index.html?kind=desktop&id=desktop&output=DP-1");
    }
}
