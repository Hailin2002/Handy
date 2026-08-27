import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Configuration
const LOCALES_DIR = path.join(__dirname, "..", "src", "i18n", "locales");
const REFERENCE_LANG = "en";

type TranslationData = Record<string, unknown>;

interface ValidationResult {
  valid: boolean;
  missing: string[][];
  extra: string[][];
}

function getLanguages(): string[] {
  const entries = fs.readdirSync(LOCALES_DIR, { withFileTypes: true });
  return entries
    .filter((entry) => entry.isDirectory() && entry.name !== REFERENCE_LANG)
    .map((entry) => entry.name)
    .sort();
}

const LANGUAGES = getLanguages();

// Colors for terminal output
const colors: Record<string, string> = {
  reset: "\x1b[0m",
  red: "\x1b[31m",
  green: "\x1b[32m",
  yellow: "\x1b[33m",
  blue: "\x1b[34m",
};

function colorize(text: string, color: string): string {
  return `${colors[color]}${text}${colors.reset}`;
}

function getAllKeyPaths(
  obj: TranslationData,
  prefix: string[] = [],
): string[][] {
  let paths: string[][] = [];
  for (const key in obj) {
    if (!Object.hasOwn(obj, key)) continue;

    const currentPath = prefix.concat([key]);
    const value = obj[key];

    if (typeof value === "object" && value !== null && !Array.isArray(value)) {
      paths = paths.concat(
        getAllKeyPaths(value as TranslationData, currentPath),
      );
    } else {
      paths.push(currentPath);
    }
  }
  return paths;
}

function hasKeyPath(obj: TranslationData, keyPath: string[]): boolean {
  let current: unknown = obj;
  for (const key of keyPath) {
    if (
      typeof current !== "object" ||
      current === null ||
      (current as Record<string, unknown>)[key] === undefined
    ) {
      return false;
    }
    current = (current as Record<string, unknown>)[key];
  }
  return true;
}

function loadTranslationFile(lang: string): TranslationData | null {
  const filePath = path.join(LOCALES_DIR, lang, "translation.json");

  try {
    const content = fs.readFileSync(filePath, "utf8");
    return JSON.parse(content) as TranslationData;
  } catch (error) {
    console.error(colorize(`✗ Error loading ${lang}/translation.json:`, "red"));
    console.error(`  ${(error as Error).message}`);
    return null;
  }
}

function validateTranslations(): void {
  console.log(colorize("\n🌍 Translation Consistency Check\n", "blue"));

  // Load reference file
  console.log(`Loading reference language: ${REFERENCE_LANG}`);
  const referenceData = loadTranslationFile(REFERENCE_LANG);

  if (!referenceData) {
    console.error(
      colorize(`\n✗ Failed to load reference file (${REFERENCE_LANG})`, "red"),
    );
    process.exit(1);
  }

  // Get all key paths from reference
  const referenceKeyPaths = getAllKeyPaths(referenceData);
  console.log(`Reference has ${referenceKeyPaths.length} keys\n`);

  // Track validation results
  let hasErrors = false;
  const results: Record<string, ValidationResult> = {};

  // Validate each language
  for (const lang of LANGUAGES) {
    const langData = loadTranslationFile(lang);

    if (!langData) {
      hasErrors = true;
      results[lang] = { valid: false, missing: [], extra: [] };
      continue;
    }

    // Find missing keys
    const missing = referenceKeyPaths.filter(
      (keyPath) => !hasKeyPath(langData, keyPath),
    );

    // Find extra keys (keys in language but not in reference)
    const langKeyPaths = getAllKeyPaths(langData);
    const extra = langKeyPaths.filter(
      (keyPath) => !hasKeyPath(referenceData, keyPath),
    );

    results[lang] = {
      valid: missing.length === 0 && extra.length === 0,
      missing,
      extra,
    };

    if (missing.length > 0 || extra.length > 0) {
      hasErrors = true;
    }
  }

  // Print results
  console.log(colorize("Results:", "blue"));
  console.log("─".repeat(60));

  for (const lang of LANGUAGES) {
    const result = results[lang];

    if (result.valid) {
      console.log(
        colorize(`✓ ${lang.toUpperCase()}: All keys present`, "green"),
      );
    } else {
      console.log(colorize(`✗ ${lang.toUpperCase()}: Issues found`, "red"));

      if (result.missing.length > 0) {
        console.log(
          colorize(`  Missing ${result.missing.length} keys:`, "yellow"),
        );
        result.missing.slice(0, 10).forEach((keyPath) => {
          console.log(`    - ${keyPath.join(".")}`);
        });
        if (result.missing.length > 10) {
          console.log(
            colorize(
              `    ... and ${result.missing.length - 10} more`,
              "yellow",
            ),
          );
        }
      }

      if (result.extra.length > 0) {
        console.log(
          colorize(
            `  Extra ${result.extra.length} keys (not in reference):`,
            "yellow",
          ),
        );
        result.extra.slice(0, 10).forEach((keyPath) => {
          console.log(`    - ${keyPath.join(".")}`);
        });
        if (result.extra.length > 10) {
          console.log(
            colorize(`    ... and ${result.extra.length - 10} more`, "yellow"),
          );
        }
      }

      console.log("");
    }
  }

  console.log("─".repeat(60));

  // Two failure modes key-parity cannot see.
  //
  // A U+FFFD REPLACEMENT CHARACTER means a file was once decoded with the
  // wrong encoding and re-saved, permanently destroying a character. That
  // happened here: 112 of them across all 24 locales, turning every ellipsis
  // into a box and German "geloescht" into "gel?scht" on screen. Key parity
  // passed the whole time, because the keys were all present — just broken.
  //
  // A key used by the code but absent from `en` renders as the literal key;
  // `common.add` was doing exactly that on the Projects page. Parity only
  // compares locales against each other, so a key missing from *all* of them
  // is invisible to it.
  const contentErrors: string[] = [];

  for (const lang of [REFERENCE_LANG, ...LANGUAGES]) {
    const raw = fs.readFileSync(
      path.join(LOCALES_DIR, lang, "translation.json"),
      "utf8",
    );
    const count = (raw.match(/�/g) ?? []).length;
    if (count > 0) {
      contentErrors.push(
        `${lang}: ${count} U+FFFD replacement character(s) — saved with a` +
          ` broken encoding, the original characters are lost`,
      );
    }
  }

  for (const missing of keysUsedInCodeButMissing()) {
    contentErrors.push(
      `${missing} is used by the app but absent from ${REFERENCE_LANG} —` +
        ` i18next renders the key itself to the user`,
    );
  }

  if (contentErrors.length > 0) {
    console.log(
      colorize(
        `
✗ Content problems:`,
        "red",
      ),
    );
    contentErrors.forEach((e) => console.log(colorize(`  ${e}`, "red")));
    hasErrors = true;
  }
  // Summary
  const validCount = Object.values(results).filter((r) => r.valid).length;
  const totalCount = LANGUAGES.length;

  if (hasErrors) {
    console.log(
      colorize(
        `\n✗ Validation failed: ${validCount}/${totalCount} languages passed`,
        "red",
      ),
    );
    process.exit(1);
  } else {
    console.log(
      colorize(
        `\n✓ All ${totalCount} languages have complete translations!`,
        "green",
      ),
    );
    process.exit(0);
  }
}

/**
 * Literal `t("a.b")` keys in src/ that the reference locale does not define.
 *
 * Only literal single-argument calls count — a key built at runtime cannot be
 * resolved statically, and guessing would produce false positives that turn the
 * check into something people switch off.
 */
function keysUsedInCodeButMissing(): string[] {
  const reference = JSON.parse(
    fs.readFileSync(
      path.join(LOCALES_DIR, REFERENCE_LANG, "translation.json"),
      "utf8",
    ),
  ) as TranslationData;

  const has = (dotted: string): boolean => {
    let node: unknown = reference;
    for (const part of dotted.split(".")) {
      if (typeof node !== "object" || node === null || !(part in node)) {
        return false;
      }
      node = (node as Record<string, unknown>)[part];
    }
    return typeof node === "string";
  };

  const files: string[] = [];
  const walk = (dir: string) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (
        /[.](ts|tsx)$/.test(entry.name) &&
        !/[.]test[.]/.test(entry.name)
      ) {
        files.push(full);
      }
    }
  };
  walk(path.join(__dirname, "..", "src"));

  const pattern =
    /\bt\(\s*["'`]([A-Za-z0-9_]+(?:[.][A-Za-z0-9_]+)+)["'`]\s*\)/g;
  const missing = new Set<string>();
  for (const file of files) {
    for (const m of fs.readFileSync(file, "utf8").matchAll(pattern)) {
      if (!has(m[1])) missing.add(m[1]);
    }
  }
  return [...missing].sort();
}
// Run validation
validateTranslations();
