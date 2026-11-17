// Generates differential fixtures from the source app's renderer.
//
// The source app is the oracle: this script calls its real formatting functions
// and writes their answers as JSON, so the Rust port can be checked against the
// source's behaviour instead of an agent re-reading the source each pass.
//
// Run from anywhere with tsx available in the source checkout:
//   cd D:/git-repos/yunari/src/web
//   npx tsx D:/git-repos/yunari-rust/scripts/parity-fixtures.mjs <out-dir> [source-repo]
//
// The output is untrusted input to the Rust tests only in the sense that it is
// generated data; the generator itself lives in this repo.

import { mkdirSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const outDir = process.argv[2];
const sourceRepo = process.argv[3] ?? "D:/git-repos/yunari";
if (!outDir) {
  console.error("usage: parity-fixtures.mjs <out-dir> [source-repo]");
  process.exit(2);
}

const barrel = join(sourceRepo, "src/web/packages/renderer-core/src/index.ts");
const core = await import(pathToFileURL(barrel).href);

const languages = ["en", "de", "fr"];
const separatorFor = (language) => {
  const parts = new Intl.NumberFormat(language, {
    useGrouping: false,
  }).formatToParts(1.5);
  return parts.find((part) => part.type === "decimal")?.value ?? ".";
};

const numbers = [];
for (const value of [
  "0",
  "-0",
  "17.64",
  "17.04",
  "17",
  "17.0",
  "1234.56",
  "-0.05",
  "0.05",
  "99.95",
  "1000000",
  "0.000001",
  "5",
  "-5.5",
  "1.25",
]) {
  const numeric = Number(value);
  for (const precision of [null, 0, 1, 2, 3, 6, 7]) {
    for (const language of languages) {
      numbers.push({
        value,
        precision,
        language,
        separator: separatorFor(language),
        expected: core.formatNumber(numeric, precision ?? undefined, language),
      });
    }
  }
}

const longUnits = { C: "Celsius", rpm: "revolutions per minute" };
const units = [];
for (const unit of ["%", "°", "C", "rpm", "km/h", "", "—"]) {
  for (const display of ["none", "short", "long"]) {
    for (const long of [null, longUnits]) {
      units.push({
        unit,
        display,
        long,
        expected: core.formatUnit(unit, display, long ?? undefined),
      });
    }
  }
}

const instants = [];
const formats = [
  "YYYY-MM-DD",
  "HH:mm",
  "H:mm",
  "h:mm A",
  "MMMM D, YYYY",
  "MMM D",
  "dddd",
  "ddd",
  "[Today is ]dddd",
  "YY/MM/DD HH:mm:ss",
  "[a]a [b]b",
  "QQ",
];
for (const value of [
  "2026-11-17T08:05:09+07:00",
  "2026-01-02T00:30:00Z",
  "2026-12-31T23:59:59Z",
  "nope",
]) {
  for (const format of formats) {
    for (const zone of [null, "America/New_York"]) {
      for (const language of ["en", "de"]) {
        instants.push({
          value,
          format,
          zone,
          language,
          expected:
            core.formatInstant(
              value,
              format,
              zone ?? undefined,
              language,
            ) ?? null,
        });
      }
    }
  }
}

const commit = execFileSync("git", ["-C", sourceRepo, "rev-parse", "HEAD"], {
  encoding: "utf8",
}).trim();

const document = {
  source: { repo: sourceRepo, commit },
  numbers,
  units,
  instants,
};

mkdirSync(outDir, { recursive: true });
const target = join(outDir, "source-format.json");
writeFileSync(target, `${JSON.stringify(document, null, 2)}\n`, "utf8");
console.log(
  `wrote ${target}: ${numbers.length} numbers, ${units.length} units, ${instants.length} instants @ ${commit.slice(0, 8)}`,
);
