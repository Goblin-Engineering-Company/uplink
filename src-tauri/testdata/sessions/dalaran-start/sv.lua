-- dalaran-start — minimal clean session (parity fixture, stage 1: raw SavedVariables).
-- start -> 2 loot -> 1 coin -> stop. No pauses, no exclusions, no folds, single build.
-- This is the byte source; envelope.json is its expected parse, resolved.json the reduction.
--
-- Streams split (LOCKED): streams.events = income FACTS only; streams.markers = lifecycle.
-- sessions = top-level sid-keyed MAP of frozen §3.3 records. _format = export envelope.
-- _registry = embedded identity registry so ch/p/item indices resolve offline.

HaulData = {
  version = 6,          -- store schemaVersion (session-model rework bumps Haul DATA_VERSION 5 -> 6)
  _gen = 1,             -- write-generation for this login

  sessions = {
    ["66961b80-1a2b"] = {
      builds = { "2026.07.16.6" },
      gameEnv = { clientBuild = "11.0.5", interface = 110005, flavor = 1 },
      schemaVersion = 6,
      startedAt = 1721160000,
      closedAt = 1721160600,
      pauses = {},                 -- LIST: [] when none
      character = 1,
      prices = {                   -- MAP: itemID -> {unit(copper), source}; frozen at close
        [124124] = { unit = 1200,  source = "tsm:dbmarket" },
        [152509] = { unit = 45000, source = "tsm:dbmarket" },
      },
      exclusions = {},             -- LIST: [] when none (NON-optional)
    },
  },

  streams = {
    events = {
      { t = 1721160010, k = "loot", sid = "66961b80-1a2b", ch = 1, p = 1,
        x = 45.20, y = 62.80, h = 180, id = 124124, count = 3,
        link = "|cffffffff|Hitem:124124::::::::70:::::|h[Cursed Queenfish]|h|r", q = 1,
        src = { t = "creature", objID = 90218 }, gen = 1 },
      { t = 1721160020, k = "loot", sid = "66961b80-1a2b", ch = 1, p = 1,
        id = 152509, count = 1,
        link = "|cff1eff00|Hitem:152509::::::::70:::::|h[Sea Stalks]|h|r", q = 2,
        src = { t = "creature", objID = 90218 }, gen = 1 },
      { t = 1721160030, k = "coin", sid = "66961b80-1a2b", ch = 1, p = 1,
        amount = 1850, src = { t = "loot" }, gen = 1 },
    },
    markers = {
      { t = 1721160000, k = "start", sid = "66961b80-1a2b", who = "Gonefishin", gen = 1 },
      { t = 1721160600, k = "stop",  sid = "66961b80-1a2b", gen = 1 },
    },
  },

  _format = {
    formatVersion = 1,
    producer = "Haul",
    producerBuild = "2026.07.16.6",
    registryVersion = 2,
    schemaVersion = 6,
    streams = {
      events  = { schema = 6 },
      markers = { schema = 6 },
    },
    generatedAt = 1721160605,
  },

  _registry = {
    char = {
      items = {
        { id = "Player-1234-0A1B2C3D", key = "Player-1234-0A1B2C3D", name = "Gonefishin",
          realm = "Stormrage", account = "Default", class = "MAGE", faction = "Alliance",
          guid = "Player-1234-0A1B2C3D", build = "2026.07.16.6",
          firstSeen = 1721159900, lastSeen = 1721160600 },
      },
      _byKey = { ["Player-1234-0A1B2C3D"] = 1 },
    },
    place = {
      items = {
        { id = "r341cmgf0y", key = "619>627", build = "2026.07.16.6",
          firstSeen = 1721159950, lastSeen = 1721160600,
          cascade = {
            { mapID = 619, name = "Broken Isles", mapType = 2, kind = "continent" },
            { mapID = 627, name = "Dalaran",      mapType = 3, kind = "zone" },
          } },
      },
      _byKey = { ["619>627"] = 1 },
    },
    item = {
      items = {
        { id = 124124, key = 124124, name = "Cursed Queenfish", quality = 1, build = "2026.07.16.6",
          firstSeen = 1721160010, lastSeen = 1721160010 },
        { id = 152509, key = 152509, name = "Sea Stalks", quality = 2, build = "2026.07.16.6",
          firstSeen = 1721160020, lastSeen = 1721160020 },
      },
      _byKey = { [124124] = 1, [152509] = 2 },
    },
  },
}
