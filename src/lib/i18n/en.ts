// English — the SOURCE dictionary. Every other language is typed against this
// file, so this is where a new string is born and where the wording is decided.
//
// Rules for editing:
//   * keys are `<area>.<thing>[.<variant>]`, lower-kebab inside a segment;
//   * the areas map to surfaces: app / nav / common / setup / home / addons /
//     data / settings / update / pair / tray / fmt / core;
//   * `core.*` are the messages that ORIGINATE IN RUST. Rust sends a code and
//     parameters; the wording lives here and only here (see msg.ts);
//   * interpolate with `{name}`; anything needing formatting goes through
//     format.ts before it reaches a placeholder;
//   * a message that varies with a number is a PluralMessage, never two keys.
//
// Text below is harvested VERBATIM from the views as of 2026-09-02, so the
// migration is a mechanical swap with no copy drift. Copy edits are a separate,
// deliberate pass.

export const en = {
  // ── app shell ──────────────────────────────────────────────────────────────
  "app.name": "Uplink",
  "app.brand": "Goblin Engineering",
  "app.load-failed": "Failed to load: {error}",
  "app.retry": "Retry",
  "app.not-paired": "not paired",
  "app.version": "v{version}",

  // ── navigation ─────────────────────────────────────────────────────────────
  "nav.home": "Home",
  "nav.addons": "Add-ons",
  "nav.data": "Data",
  "nav.settings": "Settings",

  // ── shared vocabulary ──────────────────────────────────────────────────────
  "common.save": "Save",
  "common.cancel": "Cancel",
  "common.clear": "Clear",
  "common.remove": "Remove",
  "common.rename": "Rename",
  "common.add": "Add",
  "common.refresh": "↻ Refresh",
  "common.continue": "Continue",
  "common.try-again": "Try again",
  "common.none": "—",
  "common.unplugged": "⚠ unplugged",
  "common.loading-catalog": "loading catalog…",
  "common.scanning": "scanning…",
  "common.never": "never",

  // ── setup wizard ───────────────────────────────────────────────────────────
  "setup.step": "Step {n} of {total}",
  "setup.pair.title": "Pair this device",
  "setup.pair.body":
    "Pairing links this device to your account so Uplink can sync your addon data to the website and let you into any member-only addon channels. It's optional: skip now and pair later from Settings.",
  "setup.pair.skip": "Skip for now",
  "setup.wow.title": "Point to your World of Warcraft folder",
  "setup.wow.body":
    "Uplink finds every version under your World of Warcraft folder: Retail, Classic, PTR. If your game lives on an external drive or a custom path, browse to that folder and we'll list what's inside.",
  "setup.wow.none-detected": "Nothing detected automatically. Browse to your WoW folder.",
  "setup.wow.added": "added",
  "setup.wow.browse": "＋ Browse to your WoW folder…",
  "setup.wow.skip": "Skip for now",
  "setup.wow.classic-untested": " · untested",
  "setup.addons.title": "Pick your addons",
  "setup.addons.sync-note":
    "Uplink syncs the data your installed addons record to your account. You can turn this off anytime in Settings.",
  "setup.addons.tray-check": "Start Uplink in the menu bar (no window) on launch",
  "setup.addons.tray-note":
    "Uplink lives in your menu bar (top-right on Mac) or system tray (Windows/Linux), and keeps running there after you close this window. Click its icon anytime to open Uplink or sync.",
  "setup.addons.skip": "Skip install",
  "setup.addons.finish": "Install & finish",
  "setup.addons.installing": "Installing…",
  "setup.addons.updating": "Updating Uplink…",
  "setup.addons.flavor-only": "{flavors} only",
  "setup.addons.flavor-none": "Not for this flavor",
  "setup.addons.flavor-blocked": "Not available for {flavor}",

  // ── pairing ────────────────────────────────────────────────────────────────
  "pair.getting-code": "getting a pairing code…",
  "pair.finishing": "paired, finishing up…",
  "pair.waiting": "waiting for approval…",
  "pair.denied": "That pairing was denied on the website.",
  "pair.expired": "The pairing code expired (they last 15 minutes).",
  "pair.fresh-code": "Get a fresh code",
  "pair.scan": "Scan with your phone and approve. Or approve right here:",
  "pair.open-approval": "Open approval page ↗",
  "pair.enter-code": "Or enter this code at goblineng.co/pair in any signed-in browser:",

  // ── home ───────────────────────────────────────────────────────────────────
  "home.kicker": "Welcome",
  "home.kicker-named": "Welcome, {handle}",
  "home.title": "Home",
  "home.pair-prompt": "Pair this device in Settings to see your broadcast, votes, and contribution stats.",
  "home.feed-failed": "Couldn’t load your feed:",
  "home.uploads-paused-title": "⏸ Uploads paused.",
  "home.uploads-paused-body":
    "The server is temporarily not accepting uploads. Your data is safe and will sync once it resumes.",
  "home.broadcast": "📢 Broadcast",
  "home.broadcast-dismiss": "Dismiss",
  "home.drawing-board": "The drawing board",
  "home.voting-closed": "Voting closed",
  "home.voting-closes-in": "Voting closes in",
  "home.tallying": "tallying the votes…",
  "home.votes-cast": { one: "{count} vote cast", other: "{count} votes cast" },
  "home.free-vote-expires": " · your free vote expires with this race",
  "home.in-the-workshop": "In the workshop",
  "home.your-ballot": "Your ballot:",
  "home.free-vote-available": "1 free vote",
  "home.free-vote-spent": "free vote cast ✓",
  "home.free-vote-tip": "You have a free vote this cycle. Spend it.",
  "home.free-vote-spent-tip": "Free vote cast this cycle",
  "home.boost-tip": "Boost credits: spend them as extra votes. Refunded on anything that doesn't ship.",
  "home.no-open-votes": "No open votes right now.",
  "home.top-votes": "Top votes",
  "home.votes-column": "votes",
  "home.more-votables": "+{count} more",
  "home.cast-vote": "Cast your vote ↗",
  "home.vote-open-tip": "#{number} {title} · open on goblineng.co",
  "home.vote-mine-tip": { one: "Your vote is here, including {count} boost", other: "Your vote is here, including {count} boosts" },
  "home.vote-mine-free-tip": "Your free vote is here",
  "home.profile": "Profile",
  "home.view-profile": "View profile ↗",
  "home.open-profile-tip": "Open your full profile on goblineng.co",
  "home.rank": "Rank",
  "home.rank-xp": "{rank} · XP",
  "home.discoveries-week": "Discoveries / week",
  "home.sbf": "🎣 SBF",
  "home.sbf-open-tip": "Open your fishing stats on goblineng.co",
  "home.sbf-link": "Fishing stats ↗",
  "home.fish-caught": "Fish caught",
  "home.catch-rate": "Catch rate",
  "home.fish-per-hour": "Fish / hour",
  "home.time-fished": "Time fished",
  "home.top-zone": "Top zone: {zone}",
  "home.no-zone-data": "No zone data yet",
  "home.top-fish": " · Top fish: {label} ×{count}",
  "home.sbf-week": "This week: {fish} fish / {casts} casts",
  "home.haul": "💰 Haul",
  "home.haul-open-tip": "Open your sessions on goblineng.co",
  "home.haul-link": "Sessions ↗",
  "home.haul-gold": "Haul gold",
  "home.gold-per-hour": "Gold / hour",
  "home.income": "Income",
  "home.sessions": "Sessions",
  "home.best-session": "Best session:",
  "home.currencies": "currencies",
  "home.haul-week": "This week:",
  "home.whats-new": "What’s new",
  "home.nothing-new": "Nothing new to report.",
  // The relative part comes from Intl.RelativeTimeFormat (format.ts), so this is
  // the only string: no per-unit keys, and no English "5m ago" leaking into German.
  "home.synced-ago": "synced {ago}",
  "home.synced-just-now": "synced just now",

  // ── addons ─────────────────────────────────────────────────────────────────
  "addons.kicker": "Delivery",
  "addons.title": "Addons",
  "addons.no-installs": "No WoW installs yet. Add one in Settings.",
  "addons.offline-banner": "This game folder isn’t connected right now. Its settings are kept; reconnect the drive to manage addons.",
  "addons.update-all": "⬆ {verb} all ({count})",
  "addons.update-all-tip": { one: "{verb} {count} addon on {install} · {flavor}", other: "{verb} {count} addons on {install} · {flavor}" },
  "addons.verb-install": "Install",
  "addons.verb-update": "Update",
  "addons.bulk-done": "{verb}d {done} of {total}",
  "addons.bulk-stale": "Nothing was installed. The list was stale, so it just refreshed",
  "addons.bulk-uptodate": "Everything up to date",
  "addons.no-release-for": "No release for {addon} on {channel}",
  "addons.chip-vote": "Vote",
  "addons.chip-soon": "Coming soon",
  "addons.chip-members": "Members only",
  "addons.chip-update": "update",
  "addons.chip-install-pending": "install pending",
  "addons.chip-installed": "installed",
  "addons.chip-available": "available",
  "addons.not-installed": "not installed",
  "addons.vote-for": "Vote for this addon ↗",
  "addons.learn-more": "Learn more ↗",
  "addons.votable-body": "Not on a channel you can access yet. Sign in on the site and vote to help it get built.",
  "addons.soon-body": "On the way. Open its page for details and progress.",
  "addons.locked-body": "Released on a channel you don't have access to.",
  "addons.flavor-body":
    "{addon} is built for {flavors} and won't run on {install} · {flavor}, so Uplink won't install it here. Pick a {supported} install above to get it.",
  "addons.flavor-fallback": "another WoW version",
  "addons.flavor-supported-fallback": "supported",
  "addons.view-on-site": "View on goblineng.co ↗",
  "addons.channel-tip": "Delivery channel",
  "addons.version-control": "Version control",
  "addons.channel-override": "Channel (override install default)",
  "addons.follow-install": "Follow install ({channel})",
  "addons.pin-version": "Pin version (rollback / hold)",
  "addons.latest-on-channel": "Latest on channel",
  "addons.auto-update-this": "Auto-update this addon on the schedule",
  "addons.data-sync": "Data sync",
  "addons.sync-this": "Sync this addon's data to my account",
  "addons.sync-this-note":
    "When on, Uplink uploads the data this addon records for the WoW accounts you picked on the Data page. Turn it off to keep this addon's data local.",
  "addons.update-to": "Update → {version}",
  "addons.install-version": "Install {version}",
  "addons.no-release": "No release",
  "addons.uninstall": "Uninstall",

  // ── data ───────────────────────────────────────────────────────────────────
  "data.kicker": "Data sync",
  "data.title": "Data",
  "data.accounts-title": "WoW accounts to sync",
  "data.accounts-body":
    "Pick the accounts you want to sync. Only choose accounts that are yours, because whatever you pick gets uploaded to your account. Use Rename to show a friendlier name (your real account name stays private, handy for screen recordings).",
  "data.no-installs": "No installs configured.",
  "data.install-offline": "This game folder isn’t connected right now.",
  "data.no-accounts": "No accounts found yet. Log into the game once, then check back.",
  "data.syncing-chip": "syncing",
  "data.nothing-picked": "Nothing picked, so nothing will sync here.",
  "data.rename-tip": "Rename how this account is shown, so your real account name stays out of screen recordings",
  "data.status-title": "Sync status",
  "data.waiting-chip": "{count} waiting",
  "data.clear-queue": "Clear queue",
  "data.clear-queue-tip": "A data stream is stuck and won't send (often after an addon update renamed it). Clear the stuck queue.",
  "data.full-resync": "Full resync",
  "data.full-resync-tip": "Re-send everything from the start (safe; duplicates are removed)",
  "data.sync-now": "Sync now",
  "data.sync-now-tip": "Upload now",
  "data.pair-first": "Pair this device first",
  "data.stuck-banner": "⚠ Stuck queue. This won't send on its own; click Clear queue to reset it.",
  "data.stuck-tip": "A data stream is stuck",
  "data.stuck-chip": "stuck",
  "data.stuck-stream-tip": "Stuck and won't send",
  "data.resync-note":
    "Full resync re-sends all your data from the beginning. It’s safe: the site removes duplicates, so nothing is counted twice. Click a row to see each data stream and the accounts it came from.",
  "data.col-addon": "Addon",
  "data.col-accounts": "Accounts",
  "data.col-last-upload": "Last upload",
  "data.col-uploaded": "Uploaded",
  "data.col-waiting": "Waiting",
  "data.col-stream": "Data stream",
  "data.col-account": "Account",
  "data.col-what": "What",
  "data.col-sent": "Sent",
  "data.col-new": "New",
  "data.col-result": "Result",
  "data.empty": "Nothing synced yet. Click Sync now to upload your data.",
  "data.status-failed": "Couldn’t load sync status. The error is below.",
  "data.accounts-label": "Accounts: {accounts}",
  "data.resync-confirm-title": "Re-send all your data?",
  "data.resync-confirm-body":
    "This uploads everything again from the start. It's safe: the site removes duplicates automatically, so nothing gets counted twice. Handy if something looks missing or wrong.",
  "data.purge-confirm-title": "Clear the upload queue?",
  "data.purge-confirm-body":
    "This clears the local upload queue. Use it when something stays stuck on \"waiting\" and won't send. Your uploaded data on the site is untouched, and current data re-sends on the next sync (duplicates are removed). Clear it?",
  // Data capture group. Sentences that carry <b> markup in the view are ONE
  // entry here (see context.tsx `tx`), with the emphasised words inline.
  "data.instant-capture": "Instant capture",
  "data.instant-capture-note": "(recommended)",
  "data.instant-capture-body":
    "Uplink watches your addons' saved data and uploads it seconds after the game writes it. Type /reload in game and it's on the server before you've finished your next cast. This runs even while the game is open, so it ignores the timer setting below.",
  "data.sync-while-running": "Timed check while the game is open",
  "data.sync-while-running-note": "(fallback)",
  "data.sync-while-running-body":
    "Best left on. Instant capture does not need it, but it is what catches your data if instant capture cannot start, which happens when the file watch is blocked or the drive holding your install is unplugged. Uplink notes that in the log when it happens. While instant capture is working this rarely finds anything, since WoW only writes its saved data on /reload or logout. Turn it off if you want nothing touching the disk while you play.",
  "data.interval-label": "Timed check every (seconds)",
  "data.settle-label": "Settle time before instant capture",
  "data.settle-advanced": "(advanced)",
  "data.settle-unit": "ms",
  "data.interval-note":
    "Uplink also checks when you open it and after you log out of the game. Settle time is how long Uplink waits for the game to finish writing before it reads the files; raise it if a slow drive gives you half-written uploads.",
  // Sync log group. Every finished pass lands here, whatever fired it, so the
  // trigger labels below are the point: they say whether the user asked for the
  // upload or Uplink did it on its own.
  "data.log-title": "Sync log",
  "data.log-tip": "Every sync, including the ones that run on their own",
  "data.log-none": "Nothing recorded yet",
  "data.log-empty":
    "No syncs yet. This fills in as your data uploads, whether you press Sync now or Uplink does it for you.",
  "data.log-col-when": "When",
  "data.log-col-trigger": "What started it",
  "data.log-trigger-manual": "Sync now",
  "data.log-trigger-instant": "Instant capture",
  "data.log-trigger-timed": "Timed check",
  // A skipped pass is not a failure: it ran into a reason not to run, and the reason
  // comes from Rust already worded for a person.
  "data.log-skipped": "Nothing done, {reason}",
  "data.log-nothing-new": "Nothing new to send",
  "data.log-sent": "{sent} sent, {inserted} new",
  "data.log-problems": { one: "{sent} sent, {count} problem", other: "{sent} sent, {count} problems" },
  "data.inserted-tip": "Rows the site newly stored (duplicates skipped)",
  "data.warning-tip": "Accepted, but flagged by the site",
  "data.result-ok": "ok",
  // Ditto: the table cells render Intl.RelativeTimeFormat output directly.

  // ── settings ───────────────────────────────────────────────────────────────
  "settings.kicker": "Preferences",
  "settings.title": "Settings",
  "settings.account": "Account",
  "settings.channels": "Channels: {channels}",
  "settings.unpair": "Unpair device",
  "settings.not-paired-body":
    "Not paired. Pairing links this device to your account. Scan the QR code with your phone, or approve it in a browser right here. Data sync and member channels stay off until you do.",
  "settings.pair-device": "Pair this device",
  "settings.startup": "Startup",
  "settings.launch-at-login": "Launch GEC Uplink at login",
  "settings.launch-at-login-note": "Starts Uplink automatically when you log in to your computer.",
  "settings.start-in-tray": "Start in the menu bar (don't open the window)",
  "settings.start-in-tray-note": "Launches straight to the menu-bar icon. Click it any time to open the window.",
  "settings.installs": "WoW installs",
  "settings.find-installs": "Added a new WoW version? Find installs",
  "settings.add-install": "＋ Add install",
  "settings.no-installs": "No installs yet.",
  "settings.reset-addons": "Reset addons",
  "settings.default-channel": "Default channel",
  "settings.auto-update-all": "Auto-update all addons here",
  "settings.install-note":
    "The default channel & auto-update apply to newly-added addons; the checkbox also sets every addon here now. Tune each addon on the Addons page, or Reset addons to make them all follow these defaults.",
  "settings.server": "Server",
  "settings.base-url": "Website address",
  "settings.base-url-hint": "(advanced, leave as is)",
  "settings.base-url-saved": "Base URL saved",
  "settings.server-note": "Where Uplink talks to your account. Sync options live on the Data page.",
  "settings.developer": "Developer",
  "settings.dev-url": "Manual update URL",
  "settings.dev-url-hint": "(advanced, overrides your channel)",
  "settings.dev-url-note":
    "Set this and Uplink self-updates from the URL instead of your channel, for offline or testing builds. Click Clear to go back to normal channel updates.",
  // The language control this design adds (Settings → Language).
  "settings.language": "Language",
  "settings.language-auto": "Automatic",
  "settings.language-auto-detected": "Automatic (detected: {language})",
  "settings.language-note":
    "Automatic follows your World of Warcraft client, then your computer's language. Pick one to override it.",
  "settings.language-name.en": "English",
  "settings.language-name.de": "Deutsch",
  "settings.language-help-translate": "Some of Uplink isn't translated yet, so a few things stay in English.",

  // ── self-update panel ──────────────────────────────────────────────────────
  "update.title": "Uplink updates",
  "update.current": "You're using Uplink {version}.",
  "update.flatpak-note":
    "Flatpak handles updates on the Steam Deck. Run flatpak update, or turn on automatic updates in Discover.",
  "update.check": "Check for updates",
  "update.channel": "Update channel",
  "update.daily-at": "Auto-update daily at",
  "update.auto-check": "Automatically update Uplink and flagged installs on the schedule above",
  "update.checking": "Checking…",
  "update.up-to-date": "You're up to date.",
  "update.revoked": "Your access to the {channel} channel ended. You're on Public now.",
  "update.available": "A newer version ({version}) is available.",
  "update.install-now": "Update now",
  "update.downloading": "Downloading the update…",
  "update.downloading-pct": "Downloading the update… {pct}%",
  "update.check-failed": "Couldn't check for updates right now. Please try again later.",

  // ── tray panel ─────────────────────────────────────────────────────────────
  "tray.sync-by-addon": "Sync by addon",
  "tray.nothing-synced": "Nothing synced yet.",
  "tray.queued": "⧗ {count} queued",
  "tray.synced": "✓ {count} synced",
  "tray.entries": "{count} entries",
  "tray.idle": "— idle",
  "tray.sync-now": "Sync now",
  "tray.sync-now-count": "Sync now ({count})",
  "tray.updates": "Updates",
  "tray.update-self": "Update Uplink & restart",
  "tray.update-all": "{verb} all ({count})",
  "tray.did": "{verb}d {count}",
  "tray.nothing-installed": "Nothing was installed. Refreshed the list",
  "tray.nothing-to-update": "Nothing to update",
  "tray.open-main": "Open Uplink",
  "tray.connected": "connected",
  "tray.device": "device",
  // Native OS menu / tray (built in Rust; the wording still belongs here — the
  // menu is rebuilt when the language changes).
  "tray.menu-open": "Open Uplink",
  "tray.menu-quit": "Quit GEC Uplink",
  "tray.tooltip": "GEC Uplink",

  // ── OS notifications (fired from Rust, worded here) ────────────────────────
  "notify.addon-updated": "Addon updated",
  "notify.addon-updated-body": "{folder} → {version}",
  "notify.addon-failed": "Addon update failed",
  "notify.addon-failed-body": "{addon}: {error}",
  "notify.self-update": "Uplink update",
  "notify.self-update-body": "Installing {version}…",

  // ── formatting atoms ───────────────────────────────────────────────────────
  // OPEN QUESTION for the native speaker: the German WoW client's coin letters.
  // Do NOT guess these — a wrong coin suffix is immediately, obviously wrong to a
  // German player. See the design doc, "Money".
  "fmt.gold": "g",
  "fmt.silver": "s",
  "fmt.copper": "c",
  "fmt.copper-tooltip": "{amount} copper",
  "fmt.hours": "h",
  "fmt.minutes": "m",
  "fmt.seconds": "s",
  "fmt.days": "d",

  // ── messages that ORIGINATE IN RUST ────────────────────────────────────────
  // Rust sends `{ code, params }`; these are the words. See msg.ts for the
  // code → key table and the fallback when the core is newer than the UI.
  "core.sync-already-running": "A sync is already running.",
  "core.wow-running": "WoW is running, so Uplink is waiting until you close it.",
  "core.unchanged": "Nothing changed since the last sync.",
  "core.no-exportable-data": "No data to send yet.",
  "core.nothing-new": "Nothing new.",
  "core.not-paired": "Pair this device first.",
  "core.unknown-install": "That WoW install is no longer configured.",
  "core.unknown-stream": "not used by the site yet",
  "core.empty-account": "That account name is empty.",
  "core.flavor-unsupported": "{addon} doesn't support {flavor} — it's built for {flavors}.",
  "core.addons-dir-missing": "AddOns folder not found (is the drive connected?): {path}",
  "core.not-a-zip": "That download isn't a zip archive.",
  "core.unsafe-zip-entry": "The download contains an unsafe file path: {entry}",
  "core.zip-no-addon-folder": "The download contains no addon folder.",
  "core.not-a-wow-folder":
    "That doesn't look like a WoW folder. Pick your World of Warcraft folder, a _retail_/_classic_ folder, or its Interface/AddOns folder.",
  "core.uploads-paused": "Uploads paused: {message}",
  "core.uploads-paused-default": "uploads are temporarily paused — try again later",
  "core.dead-schema": "{message} — please update {app} ({status})",
  "core.server-error": "{message} ({status})",
  "core.server-status": "The site returned an error ({status}).",
  "core.bad-response": "The site sent something Uplink couldn't read: {error}",
  "core.update-check-failed": "Update check failed (HTTP {status}).",
  "core.unexpected-pairing-status": "Unexpected pairing status: {status}",
  "core.token-clear-failed": "Couldn't clear the saved device token: {error}",
  "core.sv-not-a-table": "That SavedVariables file looks empty ({global} is not a table).",
  "core.sv-too-deep": "That SavedVariables file is nested deeper than {depth} levels, so Uplink refused it.",
  "core.warn-drained": "Batch quarantined by the legacy drain — accepted but NOT stored.",
  "core.warn-missing-stream": "Stream '{stream}' was missing from the site's reply, so its place is held.",
  "core.warn-flavor-mismatch":
    "Session {session}: this folder says {flavor} but the recorded game says {recorded} — the file may belong to a different install.",
  "core.warn-counted": "{message} ({count})",
} as const;

export type EnDictionary = typeof en;
export type MsgKey = keyof EnDictionary;
