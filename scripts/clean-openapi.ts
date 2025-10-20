#!/usr/bin/env bun
/**
 * Normalize OpenAPI JSON for progenitor codegen.
 * Steps:
 *  - Pretty-print output (openapi.pretty.json)
 *  - Replace empty media type objects ("application/json": {}) with minimal schema {"type":"object"}
 *  - Ensure any application/json media type has a schema (inject minimal object if missing)
 *  - Convert top-level response "default" referencing error response into a specific status (default 400)
 *  - Ensure at most one success (2XX) response per operation; keep lowest code
 *  - Idempotent: repeated runs yield stable output
 *
 * Usage:
 *   bun scripts/clean-openapi.ts --in openapi.json --out openapi.json --pretty openapi.pretty.json --error-status 400
 */

interface OpenAPISpec {
	[key: string]: any;
}

const args = process.argv.slice(2);
function getArg(flag: string, def?: string) {
	const idx = args.indexOf(flag);
	return idx >= 0 ? args[idx + 1] : def;
}

const inputPath = getArg("--in", "openapi.json")!;
const outputPath = getArg("--out", "openapi.json")!;
const prettyPath = getArg("--pretty", "openapi.pretty.json")!;
const errorStatus = getArg("--error-status", "400")!;

import { readFileSync, writeFileSync, existsSync } from "fs";

if (!existsSync(inputPath)) {
	console.error(`Input file ${inputPath} not found`);
	process.exit(1);
}

const spec: OpenAPISpec = JSON.parse(readFileSync(inputPath, "utf8"));
let changed = false;
const warnings: string[] = [];

const MINIMAL_OBJECT = { schema: { type: "object" } };
const HTTP_METHODS = new Set([
	"get",
	"post",
	"put",
	"patch",
	"delete",
	"options",
	"head",
]);

for (const [pathKey, pathItem] of Object.entries(spec.paths || {})) {
	for (const [method, op] of Object.entries(pathItem as any)) {
		if (!HTTP_METHODS.has(method)) continue;
		const responses = op.responses || {};
		const successCodes = Object.keys(responses).filter((c) =>
			/^2\d\d$/.test(c),
		);
		if (successCodes.length > 1) {
			const keep = successCodes.sort()[0];
			for (const code of successCodes) {
				if (code !== keep) {
					delete responses[code];
					warnings.push(
						`Dropped extra success response ${code} for ${method.toUpperCase()} ${pathKey}`,
					);
					changed = true;
				}
			}
		}
		for (const [code, resp] of Object.entries(responses)) {
			if (typeof resp !== "object" || !resp) continue;
			if (resp.content && typeof resp.content === "object") {
				const jsonContent = resp.content["application/json"];
				if (jsonContent !== undefined) {
					if (jsonContent && typeof jsonContent === "object") {
						if (Object.keys(jsonContent).length === 0) {
							resp.content["application/json"] = MINIMAL_OBJECT;
							changed = true;
						} else if (!("schema" in jsonContent)) {
							(jsonContent as any).schema = { type: "object" };
							changed = true;
						}
					} else if (jsonContent === null) {
						resp.content["application/json"] = MINIMAL_OBJECT;
						changed = true;
					}
				}
			}
			if (code === "default" && "$ref" in (resp as any)) {
				if (responses[errorStatus]) {
					delete responses[code];
					changed = true;
				} else {
					responses[errorStatus] = resp;
					delete responses[code];
					changed = true;
				}
			}
		}
		op.responses = responses;
	}
}

const prettyText = JSON.stringify(spec, null, 2) + "\n";
if (
	!existsSync(prettyPath) ||
	readFileSync(prettyPath, "utf8") !== prettyText
) {
	writeFileSync(prettyPath, prettyText);
}

if (changed) {
	writeFileSync(outputPath, JSON.stringify(spec));
}

for (const w of warnings) console.warn("Warning:", w);
console.error(
	changed ? "Cleanup complete. Changes applied." : "No changes needed.",
);
