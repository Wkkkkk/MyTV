pub struct M3uResultRow {
    pub name: String,
    pub group: String,
    pub country: String,
    pub url: String,
    pub source_kind: String,
    pub form_id: usize,
}

pub(super) async fn fetch_m3u(
    client: &reqwest::Client,
    country_code: Option<&str>,
) -> anyhow::Result<String> {
    let url = match country_code {
        Some(code) => format!("https://iptv-org.github.io/iptv/countries/{}.m3u", code),
        None => "https://iptv-org.github.io/iptv/index.m3u".to_string(),
    };
    let text = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(text)
}

pub(super) async fn url_is_reachable(client: &reqwest::Client, url: &str) -> bool {
    match client
        .head(url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) => r.status().is_success() || r.status().is_redirection(),
        Err(_) => false,
    }
}

pub(super) fn country_to_code(input: &str) -> Option<String> {
    let s = input.trim().to_lowercase();
    if s.len() == 2 && s.chars().all(|c| c.is_ascii_alphabetic()) {
        return Some(s);
    }
    let map: &[(&str, &str)] = &[
        ("afghanistan", "af"),
        ("albania", "al"),
        ("algeria", "dz"),
        ("argentina", "ar"),
        ("australia", "au"),
        ("austria", "at"),
        ("bangladesh", "bd"),
        ("belgium", "be"),
        ("brazil", "br"),
        ("bulgaria", "bg"),
        ("canada", "ca"),
        ("chile", "cl"),
        ("china", "cn"),
        ("colombia", "co"),
        ("croatia", "hr"),
        ("czech republic", "cz"),
        ("denmark", "dk"),
        ("egypt", "eg"),
        ("finland", "fi"),
        ("france", "fr"),
        ("germany", "de"),
        ("ghana", "gh"),
        ("greece", "gr"),
        ("hong kong", "hk"),
        ("hungary", "hu"),
        ("india", "in"),
        ("indonesia", "id"),
        ("iran", "ir"),
        ("iraq", "iq"),
        ("ireland", "ie"),
        ("israel", "il"),
        ("italy", "it"),
        ("japan", "jp"),
        ("jordan", "jo"),
        ("kenya", "ke"),
        ("kuwait", "kw"),
        ("lebanon", "lb"),
        ("malaysia", "my"),
        ("mexico", "mx"),
        ("morocco", "ma"),
        ("netherlands", "nl"),
        ("new zealand", "nz"),
        ("nigeria", "ng"),
        ("norway", "no"),
        ("pakistan", "pk"),
        ("philippines", "ph"),
        ("poland", "pl"),
        ("portugal", "pt"),
        ("qatar", "qa"),
        ("romania", "ro"),
        ("russia", "ru"),
        ("saudi arabia", "sa"),
        ("serbia", "rs"),
        ("singapore", "sg"),
        ("south africa", "za"),
        ("south korea", "kr"),
        ("korea", "kr"),
        ("spain", "es"),
        ("sweden", "se"),
        ("switzerland", "ch"),
        ("taiwan", "tw"),
        ("thailand", "th"),
        ("tunisia", "tn"),
        ("turkey", "tr"),
        ("ukraine", "ua"),
        ("united arab emirates", "ae"),
        ("uae", "ae"),
        ("united kingdom", "gb"),
        ("uk", "gb"),
        ("united states", "us"),
        ("usa", "us"),
        ("vietnam", "vn"),
    ];
    map.iter()
        .find(|(name, _)| s.contains(name))
        .map(|(_, code)| code.to_string())
}

/// Fetch + parse + filter the iptv-org M3U, then keep only reachable entries.
/// `limit` caps how many filtered candidates are *probed* for reachability
/// (the cap is applied before the reachability checks), so the number of rows
/// returned may be fewer than `limit`. Shared by the HTML handler and the JSON API.
pub(crate) async fn search(
    client: &reqwest::Client,
    country: &str,
    group: &str,
    limit: usize,
) -> anyhow::Result<Vec<M3uResultRow>> {
    let country_code = if country.trim().is_empty() {
        None
    } else {
        country_to_code(country)
    };
    let raw = fetch_m3u(client, country_code.as_deref()).await?;
    let all = crate::media::m3u::parse_m3u(&raw);
    let matches: Vec<_> = crate::media::m3u::filter_m3u(&all, "", group)
        .into_iter()
        .take(limit)
        .collect();

    let handles: Vec<_> = matches
        .iter()
        .map(|ch| {
            let client = client.clone();
            let url = ch.url.clone();
            tokio::spawn(async move { url_is_reachable(&client, &url).await })
        })
        .collect();
    let mut reachable = Vec::with_capacity(handles.len());
    for h in handles {
        reachable.push(h.await.unwrap_or(false));
    }

    let rows = matches
        .iter()
        .zip(reachable)
        .filter(|(_, ok)| *ok)
        .enumerate()
        .map(|(i, (ch, _))| M3uResultRow {
            name: ch.name.clone(),
            group: ch.group.clone(),
            country: ch.country.clone(),
            url: ch.url.clone(),
            source_kind: crate::model::source::SourceKind::detect(&ch.url)
                .as_str()
                .to_string(),
            form_id: i,
        })
        .collect();
    Ok(rows)
}
