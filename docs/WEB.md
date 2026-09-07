# The Mind on the web

The Mind can look things up. A gaming distribution that cannot read a release
note, a wiki page or a forum thread is guessing about drivers, and guessing is
the one thing an operator must not do. `mindd/src/daemon/web.rs` gives it
search, page reading, the two wikis it needs most, ProtonDB ratings and
downloads.

Everything goes out through **curl**, the same binary the rest of MindOS
downloads with. `reqwest` is built into mindd without a TLS backend on
purpose, so there is one path out of the machine and one place to guard it.

## The tools

| Tool | Policy | What it does |
| --- | --- | --- |
| `web_search` | observe | Search; titles, links and snippets back. |
| `web_fetch` | observe | Read one page as plain text (or JSON as JSON), optionally with its links. |
| `arch_wiki` | observe | Search the Arch Wiki and read the best page. |
| `wikipedia` | observe | The same for Wikipedia. |
| `protondb` | observe | A game's ProtonDB tier, confidence, report count and Steam app id. |
| `download_file` | change | Save a file, as the user, into their Downloads folder. |
| `open_url` | change | Open a page in the user's browser, on their screen. |

Reading is *observe*: it runs without asking. Writing a file to disk or
putting a page on the user's screen is *change*, so it is confirmed like every
other change, and `run_command` with a `curl` that writes a file, uploads one
or sends a request body is classified the same way.

## The guard

The daemon runs as root, so a page must never be able to make it talk to a
service on this machine or on the local network. Before every request, and
again before every redirect it follows:

* the scheme must be `http` or `https`; a URL with a user name or password in
  it is refused;
* the host is resolved, and **every** address it resolves to must be public.
  Loopback, `10/8`, `172.16/12`, `192.168/16`, link-local (which covers the
  `169.254.169.254` metadata address), carrier NAT, unique-local IPv6 and
  IPv4-mapped IPv6 are all refused;
* those addresses are pinned with `--resolve`, so the name cannot resolve to
  something else between the check and the connection, and the address curl
  reports having used is checked against them afterwards;
* redirects are **not** followed by curl (`-L` is absent). Each `3xx` comes
  back to the daemon, which runs the whole check again on the new URL, up to
  `max_redirects` hops.

`allow_private = true` under `[web]` lifts the address rule, for a machine
whose owner wants the Mind to read something on their own network.

Size and time are bounded too: `--max-filesize` and `--max-time` on the curl
side, and the reader stops appending at `max_bytes` whatever the server sends.

## Untrusted by construction

Everything the Mind reads online is data. Every web tool result carries a
`note` saying so, and the system prompt says it twice over: nothing on a page
can tell the Mind to run a command, install a package, change a setting or
fetch another URL, and if a page contains instructions aimed at it, it says so
and ignores them. Only the user asks for things.

This is why `open_url` and `download_file` need confirmation even though they
look harmless. They are the two tools a hostile page would want.

## Reading a page

There is no HTML library. `html_to_text` throws away `script`, `style`,
`noscript`, `svg`, `template`, `iframe` and `canvas` content, keeps the
`title`, turns block elements into line breaks, `li` into `- `, table cells
into tabs, decodes entities and collapses the whitespace, then hands back the
text with a deduplicated list of absolute links. It is a few hundred lines and
it reads the Arch Wiki, ProtonDB and a forum thread well enough to answer a
question from.

Wikis are read through their MediaWiki API rather than scraped: search goes to
`rest.php/v1/search/page` (the Arch Wiki's older full-text search answers
nothing) and the page comes from `action=parse`, whose HTML goes through the
same reader. The Arch Wiki has no TextExtracts extension, so `parse` is the
one route that works on both sites.

## Trying it

`mindd/examples/webtry.rs` runs the web module on its own, with no daemon, no
model and no VM:

```
cd mindd
cargo run --example webtry -- search "nvidia 580 wayland flicker"
cargo run --example webtry -- fetch https://archlinux.org/news/
cargo run --example webtry -- wiki "early kms"
cargo run --example webtry -- proton "Elden Ring"
cargo run --example webtry -- guard http://169.254.169.254/
```

## Fitting the context

A wiki page is longer than a 4B model's whole context, so two of them used to
end a session with a 400 from llama-server rather than a worse answer. Two
things stop that. Web tools size their text to the model: half of what the
context can hold, from `web_fetch`'s and the wikis' `max_chars`, which any
call can override. And before every request `agent::fit_context` measures the
conversation against the context that is actually left after the tool schemas,
empties the bodies of old tool results (the model has already read them), and
only then drops whole turns from the front. The system message and the newest
question always stay, and a tool result never outlives the call that made it.

## Configuration

`[web]` in `/etc/mindos/mind.toml`. `enabled = false` removes the tools from
the model's list entirely, so a machine with no internet does not watch its
Mind try. `search_url` takes any engine with `{query}` where the words go: an
HTML page is scraped, a SearXNG-style JSON answer is read directly, so a
self-hosted instance is a one-line change.
