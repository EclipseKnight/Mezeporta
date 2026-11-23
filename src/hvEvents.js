// Hunsterverse: the event badges of our server, from our REST API's
// /api/public/events/current and /api/public/festa (read by hv_live_events,
// src-tauri/src/hv_events.rs). The same answers and rules as the website's
// Events page (js/events.js of hvmhfz.eclipseforge.xyz): an event with a
// schedule row shows once its cycle has started; MezFes, the Sky Corridor,
// Raviente and the Road vote have no schedule and are always on.
//
// No imports: activeEvents.js passes the image paths through, and the unit
// tests (hvEvents.test.mjs) run it under plain node.

const FESTA_PHASES = {
  entry: ["Entry and vote", "entryEnd"],
  souls: ["Collecting souls", "soulsEnd"],
  judging: ["Judging", "judgingEnd"],
  rewards: ["Prizes open", "rewardsEnd"],
};

const DIVA_WEEKS = {
  personal: "Week 1: personal points, one Prayer Gem a day",
  tactics: "Week 2: guild week",
  rewards: "Week 3: reward lists and the coin shop",
  between: "A short pause between two weeks",
};

function ms(value) {
  const t = value ? Date.parse(value) : NaN;
  return Number.isNaN(t) ? null : t;
}

// The row's cycle has started and not ended (Diva's "between" also covers
// the gap after its last week until the next event).
function running(row, now) {
  const start = ms(row?.startsAt);
  const end = ms(row?.endsAt);
  return (start === null || start <= now) && (end === null || now < end);
}

// The answer is usable when /events/current answered with its list.
export function hasHunsterverseEvents(answer) {
  return Array.isArray(answer?.current?.events);
}

// Badge ids the API decides for Hunsterverse; the wrapper's flags still add
// the others (Pallone Festival, Conquest). Hunter Festa only when the festa
// answer came back.
export function hunsterverseDecides(answer) {
  const ids = [
    "hunting-tournament",
    "vs-bracket",
    "diva",
    "campaigns",
    "mezfes",
    "tenro",
    "raviente",
    "road-vote",
  ];
  if (answer?.festa && typeof answer.festa === "object") ids.push("hunter-festa");
  return ids;
}

function festaBadge(festa, at) {
  const festival = festa?.festival;
  const phase = FESTA_PHASES[festival?.phase];
  if (!phase) return null;
  const rows = [
    ["Status", "Live"],
    ["Phase", phase[0]],
  ];
  if (ms(festival[phase[1]]) !== null) rows.push(["Phase ends", at(festival[phase[1]])]);
  if (festival.phase === "entry" && ms(festival.soulsEnd) !== null) {
    rows.push(["Souls until", at(festival.soulsEnd)]);
  }
  return { id: "hunter-festa", label: "Hunter Festa", rows };
}

function cupBadge(row, now, at) {
  if (!row || !running(row, now)) return null;
  const quest = row.extra?.quest?.title;
  const rows = [];
  if (row.phase === "open") {
    rows.push(["Status", "Open: clear the quest as fast as you can, no sign-up"]);
    if (ms(row.phaseEndsAt) !== null) rows.push(["Clears until", at(row.phaseEndsAt)]);
  } else if (row.phase === "tallying") {
    rows.push(["Status", "Clears are closed; results are being tallied"]);
    if (ms(row.phaseEndsAt) !== null) rows.push(["Results at", at(row.phaseEndsAt)]);
  } else if (row.phase === "results") {
    rows.push(["Status", "Results are in; prizes are in the gift box"]);
    const next = row.phaseEndsAt || row.endsAt;
    if (ms(next) !== null) rows.push(["Next cup", at(next)]);
  } else {
    return null;
  }
  if (quest) rows.push(["Quest", quest]);
  return { id: "hunting-tournament", label: "Speedrun Cup", rows };
}

function vsBadge(row, now, at) {
  if (!row || !running(row, now)) return null;
  const x = row.extra ?? {};
  const rows = [];
  if (row.phase === "signup") {
    rows.push(["Status", "Sign-up open: type !vs join in chat"]);
    const until = x.signup_ends_at || row.phaseEndsAt;
    if (ms(until) !== null) rows.push(["Sign-up until", at(until)]);
  } else if (row.phase === "drawing") {
    rows.push(["Status", "Sign-up is over; the bracket is being drawn"]);
  } else if (row.phase === "round") {
    rows.push(["Status", `Round ${x.round} of ${x.rounds} is open`]);
    if (ms(row.phaseEndsAt) !== null) rows.push(["Round ends", at(row.phaseEndsAt)]);
    if (x.quest?.title) rows.push(["Quest", x.quest.title]);
  } else if (row.phase === "results") {
    rows.push(["Status", "Finished; prizes are in the gift box"]);
  } else {
    // upcoming, no_bracket (fewer than two hunters) or unknown: nothing on
    // (phases: Erupe server/channelserver/event_schedules.go).
    return null;
  }
  const entrants = Number(x.entrants) || 0;
  if (entrants) rows.push(["Entrants", String(entrants)]);
  return { id: "vs-bracket", label: "VS Bracket", rows };
}

function divaBadge(row, now, at) {
  if (!row || !running(row, now) || !DIVA_WEEKS[row.phase]) return null;
  const rows = [
    ["Status", "Live"],
    ["Now", DIVA_WEEKS[row.phase]],
  ];
  if (ms(row.phaseEndsAt) !== null) {
    rows.push([row.phase === "between" ? "Next week" : "Week ends", at(row.phaseEndsAt)]);
  }
  return { id: "diva", label: "Diva Defense", rows };
}

function campaignsBadge(row) {
  if (!row || row.phase !== "open") return null;
  const list = (row.extra?.campaigns ?? []).filter((c) => c?.title);
  if (!list.length) return null;
  return {
    id: "campaigns",
    label: "Event campaigns",
    rows: list.map((c) => [c.title, c.reward_text || "Open"]),
  };
}

const ALWAYS_ON = [
  {
    id: "mezfes",
    label: "MezFes",
    rows: [
      ["Status", "Open every day"],
      ["Tickets", "Refill each Monday, 00:00 JST"],
    ],
  },
  {
    id: "tenro",
    label: "Sky Corridor",
    rows: [
      ["Status", "Open"],
      ["What", "The Tower: climb floors, clear Duremudira, fill the town's goals"],
    ],
  },
  {
    id: "raviente",
    label: "Raviente",
    rows: [
      ["Status", "Open all the time"],
      ["Where", "Kieru in the Caravan area"],
    ],
  },
  {
    id: "road-vote",
    label: "Hunting Road vote",
    rows: [
      ["Status", "Open"],
      ["What", "Submit and vote for the next Road; a new rotation every two weeks"],
    ],
  },
];

// The badges (id, label, rows) of the events running now, time-limited ones
// first; null when the answer is not usable (the caller then uses the
// wrapper's flags). `at` formats an API time (ISO string) for the rows.
export function hunsterverseEventBadges(answer, options = {}) {
  if (!hasHunsterverseEvents(answer)) return null;
  const at =
    typeof options.formatTime === "function" ? options.formatTime : (value) => String(value);
  const now = ms(answer.current.now) ?? Date.now();
  const byEvent = {};
  for (const row of answer.current.events) {
    if (row && typeof row.event === "string") byEvent[row.event] = row;
  }
  return [
    festaBadge(answer.festa, at),
    cupBadge(byEvent.speedrun_cup, now, at),
    vsBadge(byEvent.vs_bracket, now, at),
    divaBadge(byEvent.diva, now, at),
    campaignsBadge(byEvent.campaigns),
    ...ALWAYS_ON.map((badge) => ({ ...badge, rows: badge.rows.map((r) => [...r]) })),
  ].filter(Boolean);
}
