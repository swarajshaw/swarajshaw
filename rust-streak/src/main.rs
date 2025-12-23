use chrono::{Duration, NaiveDate, Utc};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::fs;

const USERNAME: &str = "swarajshaw";

#[derive(Deserialize)]
struct User {
    public_repos: u32,
    followers: u32,
    following: u32,
}

#[derive(Deserialize)]
struct Repo {
    stargazers_count: u32,
}

#[derive(Deserialize)]
struct GraphQlResponse {
    data: Option<GraphQlData>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize)]
struct GraphQlError {
    message: String,
}

#[derive(Deserialize)]
struct GraphQlData {
    user: Option<GraphQlUser>,
}

#[derive(Deserialize)]
struct GraphQlUser {
    #[serde(rename = "contributionsCollection")]
    contributions_collection: ContributionsCollection,
}

#[derive(Deserialize)]
struct ContributionsCollection {
    #[serde(rename = "contributionCalendar")]
    contribution_calendar: ContributionCalendar,
}

#[derive(Deserialize)]
struct ContributionCalendar {
    #[serde(rename = "totalContributions")]
    total_contributions: u32,
    weeks: Vec<ContributionWeek>,
}

#[derive(Deserialize)]
struct ContributionWeek {
    #[serde(rename = "contributionDays")]
    contribution_days: Vec<ContributionDay>,
}

#[derive(Deserialize)]
struct ContributionDay {
    date: String,
    #[serde(rename = "contributionCount")]
    contribution_count: u32,
}

async fn get_json<T: for<'de> Deserialize<'de>>(
    client: &Client,
    url: String,
) -> Result<T, Box<dyn std::error::Error>> {
    let mut req = client
        .get(url)
        .header("User-Agent", "rust-github-widget")
        .header("Accept", "application/vnd.github+json");

    if let Some(token) = get_github_token() {
        req = req.header("Authorization", format!("Bearer {}", token));
    }

    let resp = req.send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API error {}: {}", status, body).into());
    }

    Ok(resp.json().await?)
}

fn get_github_token() -> Option<String> {
    for key in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(token) = std::env::var(key) {
            let token = token.trim().to_string();
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    None
}

async fn get_contribution_calendar(
    client: &Client,
) -> Result<ContributionCalendar, Box<dyn std::error::Error>> {
    let token = get_github_token()
        .ok_or("GH_TOKEN or GITHUB_TOKEN is required for GitHub GraphQL API")?;

    let query = r#"
        query($login: String!) {
          user(login: $login) {
            contributionsCollection {
              contributionCalendar {
                totalContributions
                weeks {
                  contributionDays {
                    date
                    contributionCount
                  }
                }
              }
            }
          }
        }
    "#;

    let resp = client
        .post("https://api.github.com/graphql")
        .header("User-Agent", "rust-github-widget")
        .header("Accept", "application/vnd.github+json")
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({ "query": query, "variables": { "login": USERNAME } }))
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub GraphQL error {}: {}", status, body).into());
    }

    let parsed: GraphQlResponse = resp.json().await?;
    if let Some(errors) = parsed.errors {
        let message = errors
            .into_iter()
            .map(|e| e.message)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!("GitHub GraphQL response error: {}", message).into());
    }

    let calendar = parsed
        .data
        .and_then(|d| d.user)
        .map(|u| u.contributions_collection.contribution_calendar)
        .ok_or("GitHub GraphQL response missing contribution data")?;

    Ok(calendar)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();

    // ---------- Profile ----------
    let user: User =
        get_json(&client, format!("https://api.github.com/users/{USERNAME}")).await?;

    // ---------- Repos (stars) ----------
    let repos: Vec<Repo> = get_json(
        &client,
        format!("https://api.github.com/users/{USERNAME}/repos?per_page=100"),
    )
    .await?;

    let total_stars: u32 = repos.iter().map(|r| r.stargazers_count).sum();

    // ---------- Contributions (GraphQL) ----------
    let calendar = get_contribution_calendar(&client).await?;
    let today = Utc::now().date_naive();

    let mut daily_contributions: HashMap<NaiveDate, u32> = HashMap::new();
    for week in calendar.weeks {
        for day in week.contribution_days {
            let date = NaiveDate::parse_from_str(&day.date, "%Y-%m-%d")?;
            daily_contributions.insert(date, day.contribution_count);
        }
    }

    // ---------- Current streak ----------
    let mut current_streak = 0;
    let mut d = today;
    while daily_contributions.get(&d).cloned().unwrap_or(0) > 0 {
        current_streak += 1;
        d -= Duration::days(1);
    }

    // ---------- Longest streak ----------
    let mut longest = 0;
    let mut streak = 0;
    let mut days: Vec<_> = daily_contributions
        .iter()
        .filter_map(|(d, c)| if *c > 0 { Some(*d) } else { None })
        .collect();
    days.sort();

    let mut prev = None;
    for day in days {
        if let Some(p) = prev {
            if day == p + Duration::days(1) {
                streak += 1;
            } else {
                streak = 1;
            }
        } else {
            streak = 1;
        }
        longest = longest.max(streak);
        prev = Some(day);
    }

    // ---------- Last 30 days ----------
    let mut active_30 = 0;
    let mut commits_30 = 0;
    let mut bars = String::new();

    for i in (0..30).rev() {
        let day = today - Duration::days(i);
        let count = daily_contributions.get(&day).cloned().unwrap_or(0);

        if count > 0 {
            active_30 += 1;
            commits_30 += count;
        }

        let height = (count.min(5) * 6) + 4;
        bars.push_str(&format!(
            r#"<rect x="{}" y="{}" width="4" height="{}" rx="1"/>"#,
            24 + (29 - i) * 6,
            160 - height,
            height
        ));
    }

    // ---------- SVG ----------
    let svg = format!(
        r##"
<svg width="560" height="240" viewBox="0 0 560 240" xmlns="http://www.w3.org/2000/svg">
<style>
:root {{
  --bg-start: #f7f4ef;
  --bg-end: #e0f2fe;
  --card: rgba(255,255,255,0.92);
  --text: #0f172a;
  --muted: #64748b;
  --border: rgba(15,23,42,0.08);
  --accent-1: #0ea5e9;
  --accent-2: #22c55e;
  --accent-3: #f59e0b;
}}
@media (prefers-color-scheme: dark) {{
  :root {{
    --bg-start: #0b1220;
    --bg-end: #0f172a;
    --card: rgba(15,23,42,0.88);
    --text: #e2e8f0;
    --muted: #94a3b8;
    --border: rgba(148,163,184,0.18);
    --accent-1: #38bdf8;
    --accent-2: #4ade80;
    --accent-3: #fbbf24;
  }}
}}

text {{
  font-family: "Space Grotesk", "Manrope", "Segoe UI", sans-serif;
  fill: var(--text);
}}
.small {{ fill: var(--muted); font-size: 11px; letter-spacing: 0.02em; }}
.label {{ fill: var(--muted); font-size: 10px; letter-spacing: 0.18em; }}
.value {{ font-size: 24px; font-weight: 600; }}
.title {{ font-size: 16px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; }}
.chip {{ fill: var(--text); font-size: 10px; letter-spacing: 0.14em; }}
.bar {{ fill: url(#barGrad); }}
</style>

<defs>
  <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0%" stop-color="var(--bg-start)"/>
    <stop offset="100%" stop-color="var(--bg-end)"/>
  </linearGradient>
  <linearGradient id="barGrad" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0%" stop-color="var(--accent-1)"/>
    <stop offset="100%" stop-color="var(--accent-2)"/>
  </linearGradient>
  <linearGradient id="spark" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0%" stop-color="var(--accent-1)" stop-opacity="0.9"/>
    <stop offset="100%" stop-color="var(--accent-3)" stop-opacity="0.9"/>
  </linearGradient>
  <pattern id="grid" width="22" height="22" patternUnits="userSpaceOnUse">
    <path d="M22 0H0V22" fill="none" stroke="rgba(15,23,42,0.06)" stroke-width="1"/>
  </pattern>
  <filter id="blur" x="-20%" y="-20%" width="140%" height="140%">
    <feGaussianBlur stdDeviation="18"/>
  </filter>
</defs>

<rect width="560" height="240" rx="28" fill="url(#bg)"/>
<circle cx="72" cy="40" r="54" fill="url(#spark)" opacity="0.5" filter="url(#blur)"/>
<circle cx="498" cy="196" r="64" fill="url(#spark)" opacity="0.35" filter="url(#blur)"/>

<rect x="12" y="12" width="536" height="216" rx="22" fill="var(--card)" stroke="var(--border)"/>
<rect x="12" y="12" width="536" height="216" rx="22" fill="url(#grid)" opacity="0.55"/>

<text x="32" y="38" class="title">GitHub Activity 🥷</text>
<rect x="426" y="22" width="106" height="20" rx="10" fill="url(#spark)" opacity="0.12"/>
<text x="440" y="36" class="chip">LAST 30D</text>

<text x="32" y="84" class="value">🔥 {current_streak}</text>
<text x="32" y="102" class="label">CURRENT STREAK</text>

<text x="176" y="84" class="value">🏆 {longest}</text>
<text x="176" y="102" class="label">LONGEST</text>

<text x="304" y="84" class="value">📈 {active_30}/30</text>
<text x="304" y="102" class="label">ACTIVE DAYS</text>

<rect x="32" y="126" width="496" height="62" rx="14" fill="rgba(15,23,42,0.04)"/>
<line x1="32" y1="188" x2="528" y2="188" stroke="rgba(15,23,42,0.08)" stroke-width="1"/>
<g transform="translate(32,128)">{bars}</g>

<text x="32" y="210" class="small">
Repos {repos} · Stars {stars} · Followers {followers} · Following {following} · Commits(30d) {commits_30} · Total {total_contributions}
</text>
</svg>
"##,
        current_streak = current_streak,
        longest = longest,
        active_30 = active_30,
        repos = user.public_repos,
        stars = total_stars,
        followers = user.followers,
        following = user.following,
        commits_30 = commits_30,
        total_contributions = calendar.total_contributions
    );

    fs::write("streak.svg", svg)?;
    Ok(())
}
