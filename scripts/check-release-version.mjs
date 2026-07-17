import fs from "node:fs";

const tag = process.argv[2] ?? process.env.RELEASE_TAG;

if (!tag || !/^v\d+\.\d+\.\d+$/.test(tag)) {
  throw new Error(`Expected a release tag in vMAJOR.MINOR.PATCH format, received: ${tag ?? "<empty>"}`);
}

const expected = tag.slice(1);
const packageVersion = JSON.parse(fs.readFileSync("package.json", "utf8")).version;
const tauriVersion = JSON.parse(
  fs.readFileSync("src-tauri/tauri.conf.json", "utf8"),
).version;
const cargoManifest = fs.readFileSync("src-tauri/Cargo.toml", "utf8");
const packageSection = cargoManifest.match(/^\[package\]\s*\r?\n([\s\S]*?)(?=^\[)/m)?.[1];
const cargoVersion = packageSection?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

const versions = {
  "package.json": packageVersion,
  "src-tauri/Cargo.toml": cargoVersion,
  "src-tauri/tauri.conf.json": tauriVersion,
};

const mismatches = Object.entries(versions).filter(([, version]) => version !== expected);
if (mismatches.length > 0) {
  const details = mismatches
    .map(([file, version]) => `${file}: ${version ?? "<missing>"} (expected ${expected})`)
    .join("\n");
  throw new Error(`Release versions are not synchronized:\n${details}`);
}

console.log(`Release version ${expected} matches ${Object.keys(versions).join(", ")}.`);
