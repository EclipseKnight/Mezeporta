// Unit tests of src/hvEvents.js (the launcher's Hunsterverse event badges).
//   node --test hunsterverse/tests/*.test.mjs
// The answers are shaped like the API's of 2026-11-23 (made-up quest titles).
import assert from "node:assert/strict";
import test from "node:test";
import {
  hasHunsterverseEvents,
  hunsterverseDecides,
  hunsterverseEventBadges,
} from "../../src/hvEvents.js";

const NOW = "2026-11-23T15:47:20Z";

function answer({ now = NOW, events, festa } = {}) {
  return {
    current: {
      now,
      events: events ?? [
        {
          event: "speedrun_cup",
          phase: "open",
          startsAt: "2026-11-19T15:00:00Z",
          endsAt: "2026-11-26T15:00:00Z",
          phaseEndsAt: "2026-11-25T15:00:00Z",
          extra: { quest: { title: "Speedrun Cup: Test Zinogre" } },
        },
        {
          event: "vs_bracket",
          phase: "upcoming",
          startsAt: "2026-11-26T15:00:00Z",
          endsAt: "2026-12-10T15:00:00Z",
          phaseEndsAt: "2026-11-26T15:00:00Z",
          extra: { round: 0, rounds: 0, entrants: 0 },
        },
        {
          event: "diva",
          phase: "upcoming",
          startsAt: "2026-12-21T03:00:00Z",
          endsAt: "2027-01-11T02:00:00Z",
          phaseEndsAt: "2026-12-21T03:00:00Z",
          extra: {},
        },
        {
          event: "campaigns",
          phase: "open",
          startsAt: null,
          endsAt: null,
          phaseEndsAt: null,
          extra: {
            campaigns: [
              { title: "New & Returning Hunter Pack", reward_text: "Once per hunter." },
              { title: "Weekly Supply Drop", reward_text: "Every Monday." },
            ],
          },
        },
      ],
    },
    festa:
      festa === undefined
        ? {
            festival: {
              phase: "entry",
              start: "2026-11-17T15:00:00Z",
              entryEnd: "2026-12-01T15:00:00Z",
              soulsEnd: "2026-12-22T15:00:00Z",
            },
          }
        : festa,
  };
}

const ids = (badges) => badges.map((b) => b.id);
const row = (badge, label) => badge.rows.find((r) => r[0] === label)?.[1];

test("today's answer: festa, cup, campaigns and the always-on events", () => {
  const badges = hunsterverseEventBadges(answer());
  assert.deepEqual(ids(badges), [
    "hunter-festa",
    "hunting-tournament",
    "campaigns",
    "mezfes",
    "tenro",
    "raviente",
    "road-vote",
  ]);
  const festa = badges[0];
  assert.equal(row(festa, "Phase"), "Entry and vote");
  assert.equal(row(festa, "Phase ends"), "2026-12-01T15:00:00Z");
  const cup = badges[1];
  assert.equal(cup.label, "Speedrun Cup");
  assert.equal(row(cup, "Quest"), "Speedrun Cup: Test Zinogre");
  assert.equal(row(cup, "Clears until"), "2026-11-25T15:00:00Z");
  assert.deepEqual(badges[2].rows, [
    ["New & Returning Hunter Pack", "Once per hunter."],
    ["Weekly Supply Drop", "Every Monday."],
  ]);
  assert.equal(badges.find((b) => b.id === "tenro").label, "Sky Corridor");
});

test("times go through formatTime", () => {
  const badges = hunsterverseEventBadges(answer(), { formatTime: (v) => `<${v}>` });
  assert.equal(row(badges[0], "Phase ends"), "<2026-12-01T15:00:00Z>");
});

test("VS Bracket: sign-up and rounds show, upcoming and no bracket do not", () => {
  const vs = (phase, extra = {}) =>
    hunsterverseEventBadges(
      answer({
        events: [
          {
            event: "vs_bracket",
            phase,
            startsAt: "2026-11-20T15:00:00Z",
            endsAt: "2026-12-04T15:00:00Z",
            phaseEndsAt: "2026-11-25T15:00:00Z",
            extra: { entrants: 6, signup_ends_at: "2026-11-23T20:00:00Z", ...extra },
          },
        ],
      })
    ).find((b) => b.id === "vs-bracket");
  assert.equal(row(vs("signup"), "Sign-up until"), "2026-11-23T20:00:00Z");
  assert.equal(row(vs("signup"), "Entrants"), "6");
  const round = vs("round", { round: 2, rounds: 3, quest: { title: "VS: Test" } });
  assert.equal(row(round, "Status"), "Round 2 of 3 is open");
  assert.equal(row(round, "Quest"), "VS: Test");
  assert.ok(vs("drawing"));
  assert.ok(vs("results"));
  assert.equal(vs("no_bracket"), undefined);
  assert.equal(vs("upcoming"), undefined);
});

test("Diva only while its weeks run", () => {
  const diva = (phase, now = NOW) =>
    hunsterverseEventBadges(
      answer({
        now,
        events: [
          {
            event: "diva",
            phase,
            startsAt: "2026-11-02T03:00:00Z",
            endsAt: "2026-11-23T02:00:00Z",
            phaseEndsAt: "2026-11-23T02:00:00Z",
          },
        ],
      })
    ).find((b) => b.id === "diva");
  assert.equal(row(diva("tactics", "2026-11-12T00:00:00Z"), "Now"), "Week 2: guild week");
  assert.ok(diva("between", "2026-11-16T02:30:00Z"));
  // After its last week Erupe still says "between" until the next event: off.
  assert.equal(diva("between", NOW), undefined);
  assert.equal(diva("upcoming", "2026-11-01T00:00:00Z"), undefined);
});

test("Speedrun Cup: tallying and results show, none and not started do not", () => {
  const cup = (phase, startsAt = "2026-11-19T15:00:00Z") =>
    hunsterverseEventBadges(
      answer({
        events: [
          {
            event: "speedrun_cup",
            phase,
            startsAt,
            endsAt: "2026-11-26T15:00:00Z",
            phaseEndsAt: "2026-11-25T16:00:00Z",
          },
        ],
      })
    ).find((b) => b.id === "hunting-tournament");
  assert.equal(row(cup("tallying"), "Results at"), "2026-11-25T16:00:00Z");
  assert.ok(cup("results"));
  assert.equal(cup("none"), undefined);
  assert.equal(cup("upcoming", "2026-11-24T15:00:00Z"), undefined);
});

test("festa: each phase named, no festival no badge", () => {
  const festa = (festival) =>
    hunsterverseEventBadges(answer({ festa: { festival } })).find((b) => b.id === "hunter-festa");
  assert.equal(row(festa({ phase: "souls", soulsEnd: "2026-12-22T15:00:00Z" }), "Phase"), "Collecting souls");
  assert.equal(row(festa({ phase: "rewards" }), "Phase"), "Prizes open");
  assert.equal(festa(null), undefined);
  assert.equal(festa({ phase: "none" }), undefined);
});

test("campaigns: none open, no badge", () => {
  const badges = hunsterverseEventBadges(
    answer({ events: [{ event: "campaigns", phase: "open", extra: { campaigns: [] } }] })
  );
  assert.ok(!ids(badges).includes("campaigns"));
});

test("an empty schedule still shows the always-on events", () => {
  assert.deepEqual(ids(hunsterverseEventBadges(answer({ events: [], festa: null }))), [
    "mezfes",
    "tenro",
    "raviente",
    "road-vote",
  ]);
});

test("no usable answer: null, so the wrapper's flags are used", () => {
  assert.equal(hunsterverseEventBadges(null), null);
  assert.equal(hunsterverseEventBadges(undefined), null);
  assert.equal(hunsterverseEventBadges({ current: null, festa: { festival: { phase: "entry" } } }), null);
  assert.equal(hunsterverseEventBadges({ current: { message: "error" }, festa: null }), null);
  assert.equal(hasHunsterverseEvents(answer()), true);
});

test("the wrapper still decides the festa when only the festa call failed", () => {
  assert.ok(hunsterverseDecides(answer()).includes("hunter-festa"));
  const decided = hunsterverseDecides(answer({ festa: null }));
  assert.ok(!decided.includes("hunter-festa"));
  assert.ok(decided.includes("mezfes") && decided.includes("diva"));
  assert.ok(!decided.includes("conquest") && !decided.includes("pallone-festival"));
});
