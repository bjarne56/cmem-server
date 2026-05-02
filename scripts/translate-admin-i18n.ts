#!/usr/bin/env bun
/**
 * Admin web i18n 翻译脚本 — 用 Claude Agent SDK 把 en.json 批量翻译到 29 种目标语言。
 *
 * 用法:
 *   bun scripts/translate-admin-i18n.ts                  # 翻译所有非 en/zh 语言
 *   bun scripts/translate-admin-i18n.ts ja ko de         # 只翻译指定语言
 *   bun scripts/translate-admin-i18n.ts --force          # 忽略缓存重翻
 *
 * 输入:crates/server/i18n/en.json(源 — 手写英文)
 * 输出:crates/server/i18n/{lang}.json(覆盖)
 * 缓存:scripts/.translate-admin-i18n-cache.json(避免重翻)
 *
 * 注:zh.json 是手写,不在脚本里覆盖。
 */

import { query, type SDKResultMessage } from '@anthropic-ai/claude-agent-sdk';
import * as fs from 'fs/promises';
import * as path from 'path';
import { createHash } from 'crypto';

const ROOT = path.resolve(__dirname, '..');
const I18N_DIR = path.join(ROOT, 'crates/server/i18n');
const SOURCE_PATH = path.join(I18N_DIR, 'en.json');
const CACHE_PATH = path.join(__dirname, '.translate-admin-i18n-cache.json');

// 29 个目标语言(en + zh 已手写,跳过)
const TARGET_LANGS = [
  'zh-tw', 'ja', 'ko', 'es', 'pt-br', 'fr', 'de',
  'ru', 'ar', 'he', 'pl', 'cs', 'nl', 'tr', 'uk',
  'vi', 'id', 'th', 'hi', 'bn', 'ro', 'sv', 'ur',
  'it', 'el', 'hu', 'fi', 'da', 'no',
];

const LANG_NAMES: Record<string, string> = {
  'zh-tw': 'Traditional Chinese (Taiwan)',
  'ja': 'Japanese', 'ko': 'Korean', 'es': 'Spanish',
  'pt-br': 'Brazilian Portuguese', 'fr': 'French', 'de': 'German',
  'ru': 'Russian', 'ar': 'Arabic', 'he': 'Hebrew',
  'pl': 'Polish', 'cs': 'Czech', 'nl': 'Dutch',
  'tr': 'Turkish', 'uk': 'Ukrainian', 'vi': 'Vietnamese',
  'id': 'Indonesian', 'th': 'Thai', 'hi': 'Hindi',
  'bn': 'Bengali', 'ro': 'Romanian', 'sv': 'Swedish',
  'ur': 'Urdu', 'it': 'Italian', 'el': 'Greek',
  'hu': 'Hungarian', 'fi': 'Finnish', 'da': 'Danish', 'no': 'Norwegian',
};

const CONCURRENCY = 5;
const args = process.argv.slice(2);
const force = args.includes('--force');
const targets = args.filter((a) => !a.startsWith('--'));
const langsToRun = targets.length > 0 ? targets : TARGET_LANGS;

interface CacheEntry {
  sourceHash: string;
  translation: Record<string, string>;
}
type Cache = Record<string, CacheEntry>;

async function loadCache(): Promise<Cache> {
  try {
    const text = await fs.readFile(CACHE_PATH, 'utf-8');
    return JSON.parse(text);
  } catch {
    return {};
  }
}

async function saveCache(cache: Cache): Promise<void> {
  await fs.writeFile(CACHE_PATH, JSON.stringify(cache, null, 2), 'utf-8');
}

async function loadSourceMessages(): Promise<Record<string, string>> {
  const text = await fs.readFile(SOURCE_PATH, 'utf-8');
  return JSON.parse(text);
}

function hashSource(src: Record<string, string>): string {
  const json = JSON.stringify(src, Object.keys(src).sort());
  return createHash('sha256').update(json).digest('hex');
}

function buildPrompt(targetLangName: string, source: Record<string, string>): string {
  return `Translate the following JSON object's VALUES (not keys) from English to ${targetLangName}.

Rules:
1. Keep all keys exactly as-is. Only translate values.
2. Preserve placeholders like {username}, {code} EXACTLY in their original form (with curly braces).
3. Preserve technical terms (admin, audit, observation, share, project, machine, invite) using natural ${targetLangName} equivalents in tech UIs. CSV / DB / IP / JSON / SQLite / HTMX / cookie / token / API stay English.
4. Preserve punctuation style and tone (concise admin UI labels).
5. Output VALID JSON only. No markdown fences. No commentary. Start with { and end with }.

Source JSON:
${JSON.stringify(source, null, 2)}

Output the translated JSON now:`;
}

function tryParseJson(text: string): Record<string, string> | null {
  let cleaned = text.trim();
  cleaned = cleaned.replace(/^```(?:json|JSON)?\s*\n?/m, '').replace(/\n?```\s*$/m, '');
  const start = cleaned.indexOf('{');
  const end = cleaned.lastIndexOf('}');
  if (start < 0 || end < 0) return null;
  cleaned = cleaned.slice(start, end + 1);
  try {
    const parsed = JSON.parse(cleaned);
    if (typeof parsed !== 'object' || parsed === null) return null;
    return parsed as Record<string, string>;
  } catch {
    /* fall through */
  }
  // 容错:line-by-line 提取(应付不规范的 JSON)
  const result: Record<string, string> = {};
  const re = /"([^"\\]+)"\s*:\s*"((?:[^"\\]|\\.)*)"/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(cleaned)) !== null) {
    try {
      const key = m[1];
      const val = JSON.parse(`"${m[2]}"`);
      result[key] = val;
    } catch {
      /* skip bad line */
    }
  }
  return Object.keys(result).length > 10 ? result : null;
}

async function translateOne(
  lang: string,
  source: Record<string, string>,
  cache: Cache,
  sourceHash: string,
): Promise<{ lang: string; ok: boolean; cached: boolean; cost: number; missingKeys: number }> {
  const cached = cache[lang];
  if (!force && cached && cached.sourceHash === sourceHash) {
    await writeMessagesFile(lang, cached.translation);
    return { lang, ok: true, cached: true, cost: 0, missingKeys: 0 };
  }

  const langName = LANG_NAMES[lang] || lang;
  const prompt = buildPrompt(langName, source);

  let raw = '';
  let cost = 0;
  try {
    const stream = query({
      prompt,
      options: {
        model: 'claude-haiku-4-5',
        systemPrompt:
          'You are an expert technical translator specializing in admin console UI strings. Output ONLY valid JSON, no commentary.',
        permissionMode: 'bypassPermissions',
        allowDangerouslySkipPermissions: true,
      },
    });

    for await (const message of stream) {
      if (message.type === 'assistant') {
        for (const block of message.message.content) {
          if (block.type === 'text') raw += block.text;
        }
      }
      if (message.type === 'result') {
        const r = message as SDKResultMessage;
        if (r.subtype === 'success') {
          cost = r.total_cost_usd ?? 0;
          if (!raw && r.result) raw = r.result;
        }
      }
    }
  } catch (e) {
    console.error(`[${lang}] SDK error:`, e instanceof Error ? e.message : e);
    return { lang, ok: false, cached: false, cost: 0, missingKeys: 0 };
  }

  const parsed = tryParseJson(raw);
  if (!parsed) {
    console.error(`[${lang}] Could not parse JSON from response (head 200): ${raw.slice(0, 200)}`);
    return { lang, ok: false, cached: false, cost, missingKeys: 0 };
  }

  // 检查 missing keys + 用源 fallback
  const sourceKeys = Object.keys(source);
  const merged: Record<string, string> = {};
  let missing = 0;
  for (const k of sourceKeys) {
    if (typeof parsed[k] === 'string') {
      merged[k] = parsed[k];
    } else {
      // missing → fallback 到源 en(总比缺好)
      merged[k] = source[k];
      missing += 1;
    }
  }

  cache[lang] = { sourceHash, translation: merged };
  await writeMessagesFile(lang, merged);

  return { lang, ok: true, cached: false, cost, missingKeys: missing };
}

async function writeMessagesFile(lang: string, translation: Record<string, string>): Promise<void> {
  const outPath = path.join(I18N_DIR, `${lang}.json`);
  // 用源 key 顺序写出,稳定 diff
  const ordered: Record<string, string> = {};
  for (const k of Object.keys(translation)) ordered[k] = translation[k];
  const content = JSON.stringify(ordered, null, 2) + '\n';
  await fs.writeFile(outPath, content, 'utf-8');
}

async function main(): Promise<void> {
  const source = await loadSourceMessages();
  const sourceHash = hashSource(source);
  const cache = await loadCache();

  console.log(`Source: ${SOURCE_PATH}`);
  console.log(`Keys: ${Object.keys(source).length}`);
  console.log(`Targets: ${langsToRun.length} languages`);
  console.log(`Force: ${force}`);
  console.log(`Concurrency: ${CONCURRENCY}\n`);

  let totalCost = 0;
  let okCount = 0;
  let cachedCount = 0;
  const failed: string[] = [];
  let totalMissing = 0;

  let i = 0;
  async function worker(): Promise<void> {
    while (i < langsToRun.length) {
      const idx = i++;
      const lang = langsToRun[idx];
      const startedAt = Date.now();
      const r = await translateOne(lang, source, cache, sourceHash);
      const elapsed = ((Date.now() - startedAt) / 1000).toFixed(1);
      if (r.ok) {
        okCount += 1;
        totalCost += r.cost;
        if (r.cached) cachedCount += 1;
        if (r.missingKeys > 0) totalMissing += r.missingKeys;
        console.log(
          `[${idx + 1}/${langsToRun.length}] ${r.lang.padEnd(6)} OK ${r.cached ? 'cached' : `$${r.cost.toFixed(4)}`} ${elapsed}s${r.missingKeys ? ` (missing ${r.missingKeys} keys -> fell back to en)` : ''}`,
        );
        await saveCache(cache);
      } else {
        failed.push(r.lang);
        console.log(`[${idx + 1}/${langsToRun.length}] ${r.lang.padEnd(6)} FAIL (${elapsed}s)`);
      }
    }
  }

  await Promise.all(Array.from({ length: CONCURRENCY }, () => worker()));

  await saveCache(cache);

  console.log(`\n=== Summary ===`);
  console.log(`OK:           ${okCount} / ${langsToRun.length}`);
  console.log(`Cached:       ${cachedCount}`);
  console.log(`Failed:       ${failed.length}${failed.length ? ` (${failed.join(', ')})` : ''}`);
  console.log(`Total cost:   $${totalCost.toFixed(4)}`);
  console.log(`Missing keys: ${totalMissing} (fell back to source en)`);
  console.log(`Cache:        ${CACHE_PATH}`);

  if (failed.length > 0) {
    process.exit(1);
  }
}

main().catch((e) => {
  console.error('FATAL:', e);
  process.exit(1);
});
