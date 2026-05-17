/// Full SQLite schema for Jadon's Excuses.
///
/// Migration strategy: this is a full reset of migration v1. During the
/// pre-release dev phase the previous minimal 3-table schema diverged enough
/// from the production spec (TEXT primary keys, missing foreign keys, no
/// categories table, etc.) that an additive v2 migration would have had to
/// drop and recreate most tables anyway. Since no dev databases exist in the
/// wild yet, we simply replace v1 with the full schema. The single migration
/// `tauri-plugin-sql` runs is therefore version 1 — "schema version 1" in
/// the sqlx migrations table — and seeds 6 categories plus 140 app/domain
/// mappings on first run.
pub fn initial_migration_sql() -> &'static str {
    r#"
-- ---------------------------------------------------------------------------
-- categories
-- ---------------------------------------------------------------------------
CREATE TABLE categories (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    name               TEXT UNIQUE NOT NULL,
    productivity_level TEXT NOT NULL CHECK(productivity_level IN ('focus','work','neutral','personal','distracting')),
    color              TEXT,
    is_default         INTEGER NOT NULL DEFAULT 0
);

-- ---------------------------------------------------------------------------
-- activity_events: one row per (app, window/url) focus interval
-- ---------------------------------------------------------------------------
CREATE TABLE activity_events (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    app_name         TEXT NOT NULL,
    window_title     TEXT,
    browser_url      TEXT,
    browser_domain   TEXT,
    category_id      INTEGER REFERENCES categories(id),
    started_at       INTEGER NOT NULL,
    ended_at         INTEGER,
    duration_seconds INTEGER
);

CREATE INDEX idx_activity_events_started_at  ON activity_events(started_at);
CREATE INDEX idx_activity_events_category_id ON activity_events(category_id);
CREATE INDEX idx_activity_events_app_name    ON activity_events(app_name);

-- ---------------------------------------------------------------------------
-- engagement_samples: periodic input + idle samples per activity event
-- ---------------------------------------------------------------------------
CREATE TABLE engagement_samples (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    activity_event_id     INTEGER REFERENCES activity_events(id) ON DELETE CASCADE,
    sampled_at            INTEGER NOT NULL,
    mouse_clicks          INTEGER NOT NULL DEFAULT 0,
    key_presses           INTEGER NOT NULL DEFAULT 0,
    mouse_distance_pixels INTEGER NOT NULL DEFAULT 0,
    scroll_events         INTEGER NOT NULL DEFAULT 0,
    is_idle               INTEGER NOT NULL DEFAULT 0,
    engagement_score      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_engagement_samples_event_id   ON engagement_samples(activity_event_id);
CREATE INDEX idx_engagement_samples_sampled_at ON engagement_samples(sampled_at);

-- ---------------------------------------------------------------------------
-- tab_switches: cross-app / cross-tab focus transitions
-- ---------------------------------------------------------------------------
CREATE TABLE tab_switches (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    from_app               TEXT,
    to_app                 TEXT,
    from_url               TEXT,
    to_url                 TEXT,
    switched_at            INTEGER NOT NULL,
    focus_duration_seconds INTEGER
);

CREATE INDEX idx_tab_switches_switched_at ON tab_switches(switched_at);

-- ---------------------------------------------------------------------------
-- focus_sessions: computed streaks of uninterrupted productive work
-- ---------------------------------------------------------------------------
CREATE TABLE focus_sessions (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at           INTEGER NOT NULL,
    ended_at             INTEGER NOT NULL,
    duration_seconds     INTEGER,
    primary_app          TEXT,
    avg_engagement_score INTEGER,
    switch_count         INTEGER
);

CREATE INDEX idx_focus_sessions_started_at ON focus_sessions(started_at);

-- ---------------------------------------------------------------------------
-- app_category_map: pattern-based mapping from apps/domains/titles to categories
-- ---------------------------------------------------------------------------
CREATE TABLE app_category_map (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    pattern      TEXT NOT NULL,
    pattern_type TEXT NOT NULL CHECK(pattern_type IN ('app','domain','title_contains')),
    category_id  INTEGER NOT NULL REFERENCES categories(id),
    UNIQUE(pattern, pattern_type)
);

CREATE INDEX idx_app_category_map_category_id ON app_category_map(category_id);

-- ---------------------------------------------------------------------------
-- daily_summaries: pre-aggregated per-day rollup
-- ---------------------------------------------------------------------------
CREATE TABLE daily_summaries (
    date                   TEXT PRIMARY KEY,
    total_active_seconds   INTEGER NOT NULL DEFAULT 0,
    total_idle_seconds     INTEGER NOT NULL DEFAULT 0,
    focus_score            INTEGER NOT NULL DEFAULT 0,
    longest_streak_seconds INTEGER NOT NULL DEFAULT 0,
    total_switches         INTEGER NOT NULL DEFAULT 0,
    top_app                TEXT,
    updated_at             INTEGER NOT NULL DEFAULT 0
);

-- ---------------------------------------------------------------------------
-- goals: user-defined targets that drive insights and nudges
-- ---------------------------------------------------------------------------
CREATE TABLE goals (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL,
    goal_type    TEXT NOT NULL CHECK(goal_type IN ('time','limit','streak','switch','score')),
    target_value INTEGER NOT NULL,
    category_id  INTEGER REFERENCES categories(id),
    active       INTEGER NOT NULL DEFAULT 1,
    created_at   INTEGER NOT NULL
);

-- ---------------------------------------------------------------------------
-- insights: generated narrative cards surfaced in the UI
-- ---------------------------------------------------------------------------
CREATE TABLE insights (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at INTEGER NOT NULL,
    title      TEXT NOT NULL,
    body       TEXT NOT NULL,
    severity   TEXT NOT NULL DEFAULT 'info'
);

-- ---------------------------------------------------------------------------
-- Seed: default categories.
-- IDs are deterministic on a fresh DB because we insert in this exact order:
--   1 = Deep Work, 2 = Communication, 3 = Research,
--   4 = Personal, 5 = Distracting, 6 = Uncategorized
-- ---------------------------------------------------------------------------
INSERT INTO categories (name, productivity_level, color, is_default) VALUES
    ('Deep Work',     'focus',       '#F28500', 1),
    ('Communication', 'work',        '#3B82F6', 1),
    ('Research',      'work',        '#06B6D4', 1),
    ('Personal',      'personal',    '#10B981', 1),
    ('Distracting',   'distracting', '#EF4444', 1),
    ('Uncategorized', 'neutral',     '#797D81', 1);

-- ---------------------------------------------------------------------------
-- Seed: app / domain → category mappings (140 rows total).
-- ---------------------------------------------------------------------------

-- Deep Work (apps) — 48
INSERT INTO app_category_map (pattern, pattern_type, category_id) VALUES
    ('Visual Studio Code',  'app', 1),
    ('VS Code',             'app', 1),
    ('Code',                'app', 1),
    ('Xcode',               'app', 1),
    ('Cursor',              'app', 1),
    ('IntelliJ IDEA',       'app', 1),
    ('PyCharm',             'app', 1),
    ('WebStorm',            'app', 1),
    ('Android Studio',      'app', 1),
    ('Sublime Text',        'app', 1),
    ('Vim',                 'app', 1),
    ('Neovim',              'app', 1),
    ('Emacs',               'app', 1),
    ('Microsoft Word',      'app', 1),
    ('Word',                'app', 1),
    ('Pages',               'app', 1),
    ('Microsoft Excel',     'app', 1),
    ('Excel',               'app', 1),
    ('Numbers',             'app', 1),
    ('Microsoft PowerPoint','app', 1),
    ('PowerPoint',          'app', 1),
    ('Keynote',             'app', 1),
    ('Figma',               'app', 1),
    ('Sketch',              'app', 1),
    ('Adobe Photoshop',     'app', 1),
    ('Adobe Illustrator',   'app', 1),
    ('Adobe XD',            'app', 1),
    ('Adobe Premiere Pro',  'app', 1),
    ('Final Cut Pro',       'app', 1),
    ('Logic Pro',           'app', 1),
    ('Ableton Live',        'app', 1),
    ('Notion',              'app', 1),
    ('Obsidian',            'app', 1),
    ('Bear',                'app', 1),
    ('Ulysses',             'app', 1),
    ('Scrivener',           'app', 1),
    ('Terminal',            'app', 1),
    ('iTerm',               'app', 1),
    ('iTerm2',              'app', 1),
    ('Warp',                'app', 1),
    ('Hyper',               'app', 1),
    ('Postman',             'app', 1),
    ('Insomnia',            'app', 1),
    ('TablePlus',           'app', 1),
    ('DBeaver',             'app', 1),
    ('GitHub Desktop',      'app', 1),
    ('Tower',               'app', 1),
    ('Sourcetree',          'app', 1);

-- Communication (apps) — 20
INSERT INTO app_category_map (pattern, pattern_type, category_id) VALUES
    ('Slack',           'app', 2),
    ('Microsoft Teams', 'app', 2),
    ('Teams',           'app', 2),
    ('Discord',         'app', 2),
    ('Zoom',            'app', 2),
    ('Google Meet',     'app', 2),
    ('Microsoft Outlook','app', 2),
    ('Outlook',         'app', 2),
    ('Mail',            'app', 2),
    ('Apple Mail',      'app', 2),
    ('Spark',           'app', 2),
    ('Superhuman',      'app', 2),
    ('Front',           'app', 2),
    ('Mimestream',      'app', 2),
    ('WhatsApp',        'app', 2),
    ('Telegram',        'app', 2),
    ('Signal',          'app', 2),
    ('Messages',        'app', 2),
    ('Skype',           'app', 2),
    ('Webex',           'app', 2);

-- Research (domains) — 19
INSERT INTO app_category_map (pattern, pattern_type, category_id) VALUES
    ('wikipedia.org',         'domain', 3),
    ('scholar.google.com',    'domain', 3),
    ('arxiv.org',             'domain', 3),
    ('jstor.org',             'domain', 3),
    ('semanticscholar.org',   'domain', 3),
    ('claude.ai',             'domain', 3),
    ('chatgpt.com',           'domain', 3),
    ('openai.com',            'domain', 3),
    ('anthropic.com',         'domain', 3),
    ('perplexity.ai',         'domain', 3),
    ('kagi.com',              'domain', 3),
    ('stackoverflow.com',     'domain', 3),
    ('github.com',            'domain', 3),
    ('gitlab.com',            'domain', 3),
    ('developer.mozilla.org', 'domain', 3),
    ('w3schools.com',         'domain', 3),
    ('docs.python.org',       'domain', 3),
    ('react.dev',             'domain', 3),
    ('tailwindcss.com',       'domain', 3);

-- Personal (apps + domains) — 19
INSERT INTO app_category_map (pattern, pattern_type, category_id) VALUES
    ('Spotify',             'app',    4),
    ('Apple Music',         'app',    4),
    ('music.apple.com',     'domain', 4),
    ('open.spotify.com',    'domain', 4),
    ('podcasts.apple.com',  'domain', 4),
    ('calendar.google.com', 'domain', 4),
    ('google.com/maps',     'domain', 4),
    ('maps.apple.com',      'domain', 4),
    ('chase.com',           'domain', 4),
    ('bankofamerica.com',   'domain', 4),
    ('wellsfargo.com',      'domain', 4),
    ('capitalone.com',      'domain', 4),
    ('venmo.com',           'domain', 4),
    ('paypal.com',          'domain', 4),
    ('amazon.com',          'domain', 4),
    ('target.com',          'domain', 4),
    ('walmart.com',         'domain', 4),
    ('doordash.com',        'domain', 4),
    ('ubereats.com',        'domain', 4);

-- Distracting (domains) — 31
INSERT INTO app_category_map (pattern, pattern_type, category_id) VALUES
    ('reddit.com',         'domain', 5),
    ('old.reddit.com',     'domain', 5),
    ('twitter.com',        'domain', 5),
    ('x.com',              'domain', 5),
    ('facebook.com',       'domain', 5),
    ('instagram.com',      'domain', 5),
    ('tiktok.com',         'domain', 5),
    ('youtube.com',        'domain', 5),
    ('twitch.tv',          'domain', 5),
    ('netflix.com',        'domain', 5),
    ('hulu.com',           'domain', 5),
    ('disneyplus.com',     'domain', 5),
    ('hbomax.com',         'domain', 5),
    ('max.com',            'domain', 5),
    ('primevideo.com',     'domain', 5),
    ('espn.com',           'domain', 5),
    ('bleacherreport.com', 'domain', 5),
    ('cbssports.com',      'domain', 5),
    ('theathletic.com',    'domain', 5),
    ('foxsports.com',      'domain', 5),
    ('nba.com',            'domain', 5),
    ('nfl.com',            'domain', 5),
    ('mlb.com',            'domain', 5),
    ('9gag.com',           'domain', 5),
    ('imgur.com',          'domain', 5),
    ('pinterest.com',      'domain', 5),
    ('snapchat.com',       'domain', 5),
    ('buzzfeed.com',       'domain', 5),
    ('tmz.com',            'domain', 5),
    ('dailymail.co.uk',    'domain', 5),
    ('drudgereport.com',   'domain', 5);

-- Distracting (apps) — 3 (Discord intentionally remains in Communication)
INSERT INTO app_category_map (pattern, pattern_type, category_id) VALUES
    ('Steam',               'app', 5),
    ('Epic Games Launcher', 'app', 5),
    ('Battle.net',          'app', 5);
"#
}
