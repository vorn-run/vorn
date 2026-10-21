import { createRequire as __vornCreateRequire } from 'node:module'
import { fileURLToPath as __vornFileURLToPath } from 'node:url'
import { dirname as __vornDirname } from 'node:path'
const require = __vornCreateRequire(import.meta.url)
const __vorn_filename = __vornFileURLToPath(import.meta.url)
const __vorn_dirname = __vornDirname(__vorn_filename)

var __defProp = Object.defineProperty;
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, { get: all[name], enumerable: true });
};

import { builtinModules } from "module";
var LOOPBACK_HOSTS = ["127.0.0.1", "localhost", "[::1]"];
function loopbackEndpoint(env, names, fail = (message) => new Error(message)) {
  const url = env[names.urlVar]?.trim();
  const token = env[names.tokenVar]?.trim();
  if (!url || !token) throw fail(names.missing);
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    throw fail(`${names.urlVar} is ${JSON.stringify(url)}, which is not a URL`);
  }
  if (parsed.protocol !== "http:" || !LOOPBACK_HOSTS.includes(parsed.hostname)) {
    throw fail(
      `${names.urlVar} is ${JSON.stringify(url)}; ${names.served}, over http on ${LOOPBACK_HOSTS.join(", ")}`
    );
  }
  return { url: url.replace(/\/$/, ""), token };
}
var BROWSER_HOST_ENV = "VORN_BROWSER_HOST";
var BROWSER_TOKEN_ENV = "VORN_BROWSER_TOKEN";
var SESSION_CALL_HEADER = "x-vorn-session-call";
var SessionUnavailableError = class extends Error {
  /** Asking again cannot bring the window back, so the SDK's retries let this through at once. */
  retryable = false;
  constructor(message) {
    super(message);
    this.name = "SessionUnavailableError";
  }
};
var SessionRefusedError = class extends Error {
  retryable = false;
  constructor(message) {
    super(message);
    this.name = "SessionRefusedError";
  }
};
var SESSION_TIMEOUT_MS = 45e3;
var NULL_BODY_STATUSES = /* @__PURE__ */ new Set([204, 205, 304]);
function readReply(text2) {
  let parsed;
  try {
    parsed = JSON.parse(text2);
  } catch {
    throw new Error("The signed-in window answered with a body that is not JSON");
  }
  const reply2 = parsed;
  if (!parsed || typeof parsed !== "object" || typeof reply2.status !== "number") {
    throw new Error("The signed-in window answered without a status");
  }
  return reply2;
}
var refusal = (text2) => text2.trim() || void 0;
function createSessionFetch(options = {}) {
  const env = options.env ?? process.env;
  const call = options.fetchImpl ?? fetch;
  return (async (input, init) => {
    const { url, token } = loopbackEndpoint(
      env,
      {
        urlVar: BROWSER_HOST_ENV,
        tokenVar: BROWSER_TOKEN_ENV,
        missing: `This connector acts through a signed-in Vorn window; run it from Vorn, which sets ${BROWSER_HOST_ENV}`,
        served: "the endpoint is served on this machine"
      },
      (message) => new SessionUnavailableError(message)
    );
    const request = new Request(input, init);
    const body = request.body ? await request.text() : void 0;
    const answer = await call(`${url}/fetch`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${token}`,
        "content-type": "application/json",
        ...options.call && { [SESSION_CALL_HEADER]: options.call }
      },
      body: JSON.stringify({
        url: request.url,
        method: request.method,
        headers: Object.fromEntries(request.headers),
        ...body !== void 0 && { body },
        binaryBody: true
      }),
      signal: AbortSignal.any([request.signal, AbortSignal.timeout(SESSION_TIMEOUT_MS)])
    });
    const text2 = await answer.text();
    if (answer.status === 503) {
      throw new SessionUnavailableError(
        refusal(text2) ?? "Vorn could not reach the signed-in window"
      );
    }
    if (!answer.ok) {
      throw new SessionRefusedError(
        refusal(text2) ?? `The signed-in window refused the call with HTTP ${answer.status}`
      );
    }
    const reply2 = readReply(text2);
    const content = reply2.bodyBase64 !== void 0 ? Buffer.from(reply2.bodyBase64, "base64") : reply2.body ?? "";
    return new Response(NULL_BODY_STATUSES.has(reply2.status) ? null : content, {
      status: reply2.status,
      ...reply2.headers && { headers: reply2.headers }
    });
  });
}
var PROTOCOL_VERSION = 1;
var MAX_FRAME_BYTES = 16 * 1024 * 1024;
var PROTOCOL_METHODS = {
  hello: "vorn/hello",
  manifest: "connector/manifest",
  preflight: "connector/preflight",
  options: "connector/options",
  poll: "trigger/poll",
  action: "action/run",
  footer: "extension/footer",
  handler: "extension/handler"
};
var PROTOCOL_ERROR_CODES = {
  methodNotFound: -32601,
  invalidParams: -32602,
  connectorError: -32e3,
  unsupportedProtocol: -32001,
  beforeHello: -32002
};
var RETRYABLE_STATUS = /* @__PURE__ */ new Set([408, 425, 429, 500, 502, 503, 504]);
var DEFAULT_ATTEMPTS = 3;
var DEFAULT_BASE_DELAY_MS = 250;
var DEFAULT_MAX_DELAY_MS = 3e4;
var MAX_ATTEMPTS = 10;
var MAX_TOTAL_WAIT_MS = 12e4;
var wait = (ms) => new Promise((resolve4) => {
  setTimeout(resolve4, ms);
});
function retryAfterMs(header, now) {
  if (!header) return void 0;
  const trimmed = header.trim();
  if (trimmed === "") return void 0;
  const seconds = Number(trimmed);
  if (Number.isFinite(seconds)) return Math.max(0, seconds * 1e3);
  const at = Date.parse(trimmed);
  if (Number.isNaN(at)) return void 0;
  return Math.max(0, at - now);
}
function backoffMs(attempt, policy = {}) {
  const base = policy.baseDelayMs ?? DEFAULT_BASE_DELAY_MS;
  const max = policy.maxDelayMs ?? DEFAULT_MAX_DELAY_MS;
  return Math.min(max, base * 2 ** attempt);
}
function isFinal(error) {
  return error?.retryable === false;
}
function resilientFetch(options) {
  const attempts = Math.min(MAX_ATTEMPTS, Math.max(1, options.retry?.attempts ?? DEFAULT_ATTEMPTS));
  const sleep = options.sleep ?? wait;
  const ceiling = options.retry?.maxDelayMs ?? DEFAULT_MAX_DELAY_MS;
  const send2 = async (input, init) => {
    let waited = 0;
    const pause = async (ms) => {
      if (waited + ms > MAX_TOTAL_WAIT_MS) return false;
      waited += ms;
      await sleep(ms);
      return true;
    };
    for (let attempt = 0; attempt < attempts; attempt++) {
      const last = attempt === attempts - 1;
      try {
        const response = await options.fetchImpl(input, init);
        if (!RETRYABLE_STATUS.has(response.status)) return response;
        if (!options.retryable || last) return response;
        const asked = retryAfterMs(response.headers.get("retry-after"), Date.now());
        const delay = asked === void 0 ? backoffMs(attempt, options.retry) : Math.min(asked, ceiling);
        if (!await pause(delay)) return response;
      } catch (error) {
        if (!options.retryable || last || isFinal(error)) throw error;
        if (!await pause(backoffMs(attempt, options.retry))) throw error;
      }
    }
    throw new Error("Request was never attempted");
  };
  return send2;
}
var ActionArgumentError = class extends Error {
  field;
  constructor(field, message) {
    super(message);
    this.name = "ActionArgumentError";
    this.field = field;
  }
};
var UpstreamStatusError = class extends Error {
  status;
  viaSession;
  constructor(status, message, viaSession = false) {
    super(message);
    this.name = "UpstreamStatusError";
    this.status = status;
    this.viaSession = viaSession;
  }
};
var UnknownNameError = class extends Error {
  constructor(message) {
    super(message);
    this.name = "UnknownNameError";
  }
};
var messageOf = (error) => error instanceof Error ? error.message : String(error);
function* causes(error) {
  let at = error;
  for (let depth = 0; at !== void 0 && depth < 8; depth++) {
    yield at;
    at = at instanceof Error ? at.cause : void 0;
  }
}
function protocolError(error) {
  const failed = (data) => ({
    code: PROTOCOL_ERROR_CODES.connectorError,
    message: messageOf(error),
    data
  });
  for (const at of causes(error)) {
    if (at instanceof ActionArgumentError) return failed({ kind: "validation", field: at.field });
    if (at instanceof SessionUnavailableError) {
      return failed({ kind: "app-offline", retryable: false });
    }
    if (at instanceof UpstreamStatusError) {
      if (at.viaSession && (at.status === 401 || at.status === 403)) {
        return failed({ kind: "signed-out", retryable: false });
      }
      return failed({ kind: "upstream", retryable: RETRYABLE_STATUS.has(at.status) });
    }
  }
  return failed({ kind: "internal" });
}
var ORIGIN_PATTERN = /^https:\/\/(\*\.)?[a-z0-9-]+(\.[a-z0-9-]+)+$/i;
function withinOrigins(origins, url) {
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  if (parsed.protocol !== "https:" || parsed.port !== "") return false;
  const target = parsed.hostname.toLowerCase();
  return origins.some((origin) => {
    if (!ORIGIN_PATTERN.test(origin)) return false;
    const wildcard = origin.startsWith("https://*.");
    const host = origin.slice(wildcard ? "https://*.".length : "https://".length).toLowerCase();
    return wildcard ? target.endsWith(`.${host}`) : target === host;
  });
}
var FORBIDDEN_SESSION_HEADERS = /* @__PURE__ */ new Set([
  "cookie",
  "cookie2",
  "authorization",
  "host",
  "origin",
  "referer",
  "content-length"
]);
var HEADER_NAME = /^[A-Za-z0-9!#$%&'*+.^_`|~-]+$/;
function allowedSessionHeader(name) {
  const lower = name.toLowerCase();
  return HEADER_NAME.test(name) && !FORBIDDEN_SESSION_HEADERS.has(lower) && !lower.startsWith("sec-") && !lower.startsWith("proxy-");
}
var UNSAFE_KEYS = /* @__PURE__ */ new Set(["__proto__", "constructor", "prototype"]);
function isRecord(value2) {
  return typeof value2 === "object" && value2 !== null && !Array.isArray(value2);
}
function segments(path2) {
  return path2.split(".").map((part) => part.trim()).filter((part) => part !== "");
}
function valueAt(value2, path2) {
  let current = value2;
  for (const key2 of segments(path2)) {
    if (UNSAFE_KEYS.has(key2)) return void 0;
    if (Array.isArray(current)) {
      const index = Number(key2);
      if (!Number.isInteger(index)) return void 0;
      current = current[index];
      continue;
    }
    if (!isRecord(current)) return void 0;
    current = Object.prototype.hasOwnProperty.call(current, key2) ? current[key2] : void 0;
  }
  return current;
}
function withValueAt(value2, path2, next) {
  const keys = segments(path2);
  if (keys.length === 0) return next;
  const [head, ...rest] = keys;
  if (UNSAFE_KEYS.has(head)) return value2;
  if (Array.isArray(value2)) {
    const index = Number(head);
    if (!Number.isInteger(index)) return value2;
    const copy = [...value2];
    copy[index] = rest.length === 0 ? next : withValueAt(copy[index], rest.join("."), next);
    return copy;
  }
  const base = isRecord(value2) ? value2 : {};
  return {
    ...base,
    [head]: rest.length === 0 ? next : withValueAt(base[head], rest.join("."), next)
  };
}
function pick(value2, keys) {
  if (Array.isArray(value2)) return value2.map((entry) => pick(entry, keys));
  if (!isRecord(value2)) return value2;
  const out = {};
  for (const key2 of keys) {
    if (UNSAFE_KEYS.has(key2)) continue;
    if (Object.prototype.hasOwnProperty.call(value2, key2)) out[key2] = value2[key2];
  }
  return out;
}
function rename(value2, from, to) {
  if (Array.isArray(value2)) return value2.map((entry) => rename(entry, from, to));
  if (!isRecord(value2)) return value2;
  if (UNSAFE_KEYS.has(from) || UNSAFE_KEYS.has(to)) return value2;
  if (!Object.prototype.hasOwnProperty.call(value2, from)) return value2;
  const out = {};
  for (const [key2, entry] of Object.entries(value2)) {
    if (key2 === from) out[to] = entry;
    else if (key2 !== to) out[key2] = entry;
  }
  return out;
}
function applyOp(value2, op) {
  if (op.op === "flatten") return valueAt(value2, op.path);
  const target = op.path === void 0 ? value2 : valueAt(value2, op.path);
  if (op.path !== void 0 && target === void 0) return value2;
  let next;
  if (op.op === "pick") next = pick(target, op.keys);
  else if (op.op === "rename") next = rename(target, op.from, op.to);
  else if (op.op === "filter") {
    next = Array.isArray(target) ? target.filter((entry) => isRecord(entry) && valueAt(entry, op.key) === op.equals) : target;
  } else {
    next = Array.isArray(target) ? target.map((entry) => applyPostReceive(entry, op.ops)) : target;
  }
  return op.path === void 0 ? next : withValueAt(value2, op.path, next);
}
function applyPostReceive(value2, ops) {
  return (ops ?? []).reduce(applyOp, value2);
}
var KEY_PATTERN = /^[a-zA-Z][a-zA-Z0-9_-]*$/;
var PATH_DATA_PATTERN = /^[MmZzLlHhVvCcSsQqTtAa0-9\s,.\-+eE]+$/;
var VIEW_BOX_PATTERN = /^-?[\d.]+\s+-?[\d.]+\s+-?[\d.]+\s+-?[\d.]+$/;
var DEDUPE_STRATEGIES = ["timestamp", "lastItem"];
var AUTH_RUNGS = ["none", "cli", "key", "browser", "oauth"];
var EXTENSION_AGENTS = [
  "claude",
  "copilot",
  "codex",
  "opencode",
  "gemini",
  "shell"
];
var ABSOLUTE_URL_PATTERN = /^https?:\/\//i;
var CONFIG_ROOTED_URL_PATTERN = /^\{\{\s*config\./;
function assertUnique(kind, keys) {
  const seen = /* @__PURE__ */ new Set();
  for (const key2 of keys) {
    if (seen.has(key2)) throw new Error(`Duplicate ${kind} "${key2}"`);
    seen.add(key2);
  }
}
function assertAuth(definition) {
  const auth = definition.auth;
  if (!auth) return;
  const id = definition.id;
  if (!AUTH_RUNGS.includes(auth.rung)) {
    throw new Error(
      `Connector ${id} declares unknown auth rung ${JSON.stringify(auth.rung)}; expected ${AUTH_RUNGS.join(", ")}`
    );
  }
  if (auth.rung === "cli" && !auth.probe?.command?.trim()) {
    throw new Error(`Connector ${id} borrows a CLI login but declares no probe command to ask it`);
  }
  if (auth.rung === "key") {
    const keys = auth.keys ?? [];
    if (keys.length === 0) {
      throw new Error(`Connector ${id} signs in with a key but names no config field holding it`);
    }
    const declared = new Set((definition.config ?? []).map((field) => field.key));
    for (const key2 of keys) {
      if (!declared.has(key2)) {
        throw new Error(`Connector ${id} names auth key "${key2}", which is not a config field`);
      }
    }
  }
  if (auth.rung === "browser") assertBrowserSignIn(id, auth.browser);
  if (auth.rung === "none" || auth.rung === "browser") {
    const secret = (definition.config ?? []).find((field) => field.secret === true);
    if (secret) {
      const claim = auth.rung === "none" ? "needs no sign-in" : "signs in through a Vorn window";
      throw new Error(
        `Connector ${id} claims it ${claim} but declares secret field "${secret.key}"`
      );
    }
  }
}
function assertBrowserSignIn(id, browser) {
  if (!browser) {
    throw new Error(
      `Connector ${id} signs in through a Vorn window but declares no browser sign-in`
    );
  }
  const origins = Array.isArray(browser.origins) ? browser.origins : [];
  const bad = origins.find((origin) => typeof origin !== "string" || !ORIGIN_PATTERN.test(origin));
  if (origins.length === 0 || bad !== void 0) {
    throw new Error(
      `Connector ${id} must name its origins as https://host or https://*.host` + (bad !== void 0 ? `; ${JSON.stringify(bad)} is neither` : "")
    );
  }
  const places = [
    ["sign-in page", browser.signInUrl],
    ["signed-in check", browser.check?.url]
  ];
  for (const [what, url] of places) {
    if (typeof url !== "string" || !withinOrigins(origins, url)) {
      throw new Error(
        `Connector ${id} puts its ${what} ${JSON.stringify(url ?? "")} outside its origins`
      );
    }
  }
  const identity = browser.check?.identity;
  if (!Array.isArray(identity) || identity.some((path2) => typeof path2 !== "string" || !path2.trim())) {
    throw new Error(`Connector ${id} must name its identity fields as non-empty strings`);
  }
  const headers = browser.check?.headers;
  if (headers === void 0) return;
  const badHeader = isRecord(headers) ? Object.entries(headers).find(
    ([name, value2]) => typeof value2 !== "string" || !allowedSessionHeader(name)
  ) : void 0;
  if (!isRecord(headers) || badHeader) {
    throw new Error(
      `Connector ${id} may add only plain string headers to its signed-in check` + (badHeader ? `; ${JSON.stringify(badHeader[0])} is not one` : "")
    );
  }
}
function assertIdentity(kind, definition) {
  if (!KEY_PATTERN.test(definition.id ?? "")) {
    throw new Error(`${kind} id "${definition.id}" must start with a letter and be url-safe`);
  }
  if (!definition.name?.trim()) {
    throw new Error(`${kind} ${definition.id} is missing a name`);
  }
  assertIcon(`${kind} ${definition.id}`, definition.icon);
}
function assertIcon(subject, icon2) {
  if (!icon2) return;
  const { viewBox, paths } = icon2;
  if (!Array.isArray(paths) || paths.length === 0) {
    throw new Error(`${subject} has an icon with no paths`);
  }
  for (const path2 of paths) {
    if (typeof path2 !== "string" || !PATH_DATA_PATTERN.test(path2)) {
      throw new Error(
        `${subject} has an icon path that is not SVG path data. Only path data is accepted, not markup.`
      );
    }
  }
  if (viewBox !== void 0 && !VIEW_BOX_PATTERN.test(viewBox)) {
    throw new Error(`${subject} has an icon viewBox that is not four numbers`);
  }
}
function envNameFor(key2, explicit) {
  if (explicit) return explicit;
  return key2.replace(/([a-z0-9])([A-Z])/g, "$1_$2").replace(/[-\s]+/g, "_").toUpperCase();
}
function defineConnector(definition) {
  assertIdentity("Connector", definition);
  const triggers = definition.triggers ?? [];
  const actions = definition.actions ?? [];
  if (triggers.length === 0 && actions.length === 0) {
    throw new Error(`Connector ${definition.id} declares no triggers and no actions`);
  }
  for (const trigger of triggers) {
    if (!KEY_PATTERN.test(trigger.type ?? "")) {
      throw new Error(`Trigger type "${trigger.type}" must start with a letter and be url-safe`);
    }
    const loose = trigger;
    const declarative = typeof loose.fetch === "function";
    const imperative = typeof loose.poll === "function";
    if (declarative && imperative) {
      throw new Error(`Trigger ${trigger.type} declares both fetch() and poll(); pick one`);
    }
    if (declarative !== (loose.dedupe !== void 0)) {
      throw new Error(
        `Trigger ${trigger.type} needs fetch() and a dedupe strategy together, not one alone`
      );
    }
    if (loose.dedupe !== void 0 && !DEDUPE_STRATEGIES.includes(loose.dedupe)) {
      throw new Error(
        `Trigger ${trigger.type} has unknown dedupe strategy ${JSON.stringify(loose.dedupe)}; expected ${DEDUPE_STRATEGIES.join(" or ")}`
      );
    }
    if (loose.poll !== void 0 && !imperative) {
      throw new Error(`Trigger ${trigger.type} declares poll but it is not a function`);
    }
    if (!declarative && !imperative) {
      throw new Error(`Trigger ${trigger.type} is missing a fetch() or poll() implementation`);
    }
  }
  for (const action of actions) {
    if (!KEY_PATTERN.test(action.type ?? "")) {
      throw new Error(`Action type "${action.type}" must start with a letter and be url-safe`);
    }
    const loose = action;
    const written = typeof loose.run === "function";
    const declared = loose.request !== void 0;
    if (written && declared) {
      throw new Error(`Action ${action.type} declares both run() and a request; pick one`);
    }
    if (!written && !declared) {
      throw new Error(`Action ${action.type} is missing a run() implementation or a request`);
    }
    if (declared) {
      const request = loose.request;
      if (typeof request?.url !== "string" || request.url.trim() === "") {
        throw new Error(`Action ${action.type} declares a request with no URL`);
      }
      const url = request.url.trim();
      if (!ABSOLUTE_URL_PATTERN.test(url) && !CONFIG_ROOTED_URL_PATTERN.test(url)) {
        throw new Error(
          `Action ${action.type} declares the request URL "${url}", which is neither absolute nor rooted in a {{config.\u2026}} value`
        );
      }
    }
    if (!declared && loose.postReceive !== void 0) {
      throw new Error(`Action ${action.type} has postReceive but no request for it to reshape`);
    }
    for (const input of action.inputs ?? []) {
      if (input.loadOptions !== void 0 && definition.options?.[input.loadOptions] === void 0) {
        throw new Error(
          `Action ${action.type} argument "${input.key}" loads options from "${input.loadOptions}", which the connector does not serve`
        );
      }
    }
  }
  assertUnique(
    "trigger",
    triggers.map((trigger) => trigger.type)
  );
  assertUnique(
    "action",
    actions.map((action) => action.type)
  );
  assertUnique(
    "config field",
    (definition.config ?? []).map((field) => field.key)
  );
  assertAuth(definition);
  return {
    ...definition,
    kind: "connector",
    version: definition.version ?? "0.0.0",
    config: definition.config ?? [],
    triggers,
    actions
  };
}
function resolveConfig(connector2, env = process.env) {
  const config = {};
  const missing = [];
  for (const field of connector2.config) {
    const name = envNameFor(field.key, field.env);
    const value2 = env[name] ?? field.default;
    if (value2 === void 0 || value2 === "") {
      if (field.required) missing.push(`${field.key} (${name})`);
      continue;
    }
    config[field.key] = value2;
  }
  if (missing.length > 0) {
    throw new Error(
      `Connector ${connector2.id} is missing required configuration: ${missing.join(", ")}`
    );
  }
  return config;
}
function connectionSetup(connector2, triggerType) {
  const trigger = connector2.triggers.find((entry) => entry.type === triggerType);
  if (!trigger) {
    throw new Error(`Connector ${connector2.id} has no trigger "${triggerType}"`);
  }
  return {
    connectorId: connector2.id,
    triggerType,
    env: connector2.config.map((field) => ({
      name: envNameFor(field.key, field.env),
      required: field.required === true,
      secret: field.secret === true,
      ...field.description !== void 0 && { description: field.description },
      ...field.builderHint !== void 0 && { builderHint: field.builderHint }
    }))
  };
}
function manifestContributions(connector2) {
  const contributes = connector2.contributes;
  if (!contributes) return void 0;
  const shared = (contribution) => ({
    id: contribution.id,
    title: contribution.title,
    ...contribution.description !== void 0 && { description: contribution.description },
    ...contribution.when !== void 0 && { when: contribution.when }
  });
  return {
    ...contributes.panes !== void 0 && {
      panes: contributes.panes.map((pane) => ({
        ...shared(pane),
        ...pane.icon !== void 0 && { icon: pane.icon },
        ...pane.web !== void 0 && { web: pane.web },
        ...pane.command !== void 0 && { command: pane.command }
      }))
    },
    ...contributes.footers !== void 0 && {
      footers: contributes.footers.map((footer) => ({ ...shared(footer), every: footer.every }))
    },
    ...contributes.linkHandlers !== void 0 && {
      linkHandlers: contributes.linkHandlers.map((handler) => ({
        ...shared(handler),
        pattern: handler.pattern,
        example: handler.example
      }))
    }
  };
}
function connectorManifest(connector2) {
  const contributes = manifestContributions(connector2);
  return {
    protocol: PROTOCOL_VERSION,
    id: connector2.id,
    name: connector2.name,
    version: connector2.version,
    kind: connector2.kind,
    ...connector2.description !== void 0 && { description: connector2.description },
    ...connector2.icon !== void 0 && { icon: connector2.icon },
    ...connector2.auth !== void 0 && { auth: connector2.auth },
    ...contributes !== void 0 && { contributes },
    ...connector2.permissions !== void 0 && { permissions: connector2.permissions },
    ...connector2.activates !== void 0 && { activates: connector2.activates },
    triggers: connector2.triggers.map((trigger) => ({
      type: trigger.type,
      label: trigger.label,
      ...trigger.description !== void 0 && { description: trigger.description },
      // Carried through so the app can seed a connection's status mapping and
      // its polling workflow. Absent when the connector said nothing, which is
      // different from saying there is nothing.
      ...trigger.statusMapping !== void 0 && { statusMapping: trigger.statusMapping },
      ...trigger.defaultWorkflow !== void 0 && { defaultWorkflow: trigger.defaultWorkflow },
      setup: connectionSetup(connector2, trigger.type)
    })),
    actions: connector2.actions.map((action) => ({
      type: action.type,
      label: action.label,
      ...action.description !== void 0 && { description: action.description },
      ...action.idempotent !== void 0 && { idempotent: action.idempotent },
      inputs: (action.inputs ?? []).map((input) => ({
        key: input.key,
        label: input.label,
        type: input.type ?? "string",
        required: input.required === true,
        ...input.description !== void 0 && { description: input.description },
        ...input.options !== void 0 && { options: input.options },
        ...input.loadOptions !== void 0 && { loadOptions: input.loadOptions },
        ...input.builderHint !== void 0 && { builderHint: input.builderHint }
      })),
      ...action.outputs !== void 0 && { outputs: action.outputs },
      ...action.sample !== void 0 && { sample: action.sample }
    }))
  };
}
var RECORDED_CHECKS = [
  "manifest",
  "auth",
  "secrets",
  "actions",
  "dedupe",
  "no-lifecycle-scripts",
  "keywords",
  "no-runtime-deps",
  "mock",
  "live",
  "launch",
  "contributes",
  "footers",
  "handlers",
  "permissions"
];
function selfCheck(manifest, receipt, known) {
  const problems = [];
  const r = receipt;
  if (!r || typeof r !== "object") return ["verified.json is missing or is not an object"];
  if (r.version !== manifest.version) {
    problems.push(
      `verified.json records ${String(r.version)}, and the bundle is ${manifest.version}`
    );
  }
  if (!Array.isArray(r.checks) || r.checks.length === 0) {
    problems.push("verified.json records no checks");
  } else {
    for (const check of r.checks) {
      if (!known.includes(String(check)))
        problems.push(`verified.json records an unknown check ${String(check)}`);
    }
  }
  return problems;
}
function lineReader(onLine, overflow) {
  let pending = [];
  let size = 0;
  return (chunk) => {
    let start = 0;
    for (let end = chunk.indexOf(10); end !== -1; end = chunk.indexOf(10, start)) {
      const part = chunk.subarray(start, end);
      if (size + part.length > MAX_FRAME_BYTES) return overflow();
      const whole = pending.length === 0 ? part : Buffer.concat([...pending, part], size + part.length);
      const line = whole.toString("utf8");
      pending = [];
      size = 0;
      start = end + 1;
      onLine(line.endsWith("\r") ? line.slice(0, -1) : line);
    }
    const rest = chunk.subarray(start);
    size += rest.length;
    if (size > MAX_FRAME_BYTES) return overflow();
    if (rest.length > 0) pending.push(rest);
  };
}
var MAX_PACK_BYTES = 8 * 1024 * 1024;
var MAX_UNPACKED_BYTES = 32 * 1024 * 1024;
var BUILTINS = new Set(builtinModules);
var BEFORE_REGEX = /* @__PURE__ */ new Set(["", ..."(,=:[!&|?{};+-*%~^<>"]);
var HOST_URL_ENV = "VORN_EXTENSION_HOST";
var HOST_TOKEN_ENV = "VORN_EXTENSION_TOKEN";
var PermissionDeniedError = class extends Error {
  constructor(method, detail) {
    super(`The host refused ${method}: ${detail}`);
    this.name = "PermissionDeniedError";
  }
};
var HostReplyError = class extends Error {
  constructor(method, detail) {
    super(`The host answered ${method} with ${detail}`);
    this.name = "HostReplyError";
  }
};
var HOST_TIMEOUT_MS = 15e3;
function endpoint(env) {
  return loopbackEndpoint(env, {
    urlVar: HOST_URL_ENV,
    tokenVar: HOST_TOKEN_ENV,
    missing: `This extension was started without a host bridge; ${HOST_URL_ENV} and ${HOST_TOKEN_ENV} are set by Vorn`,
    served: "the bridge is served on this machine"
  });
}
function createExtensionHost(options) {
  const env = options.env ?? process.env;
  const call = options.fetchImpl ?? fetch;
  async function ask(method, params) {
    const { url, token } = endpoint(env);
    const response = await call(`${url}/${method}`, {
      method: "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
      body: JSON.stringify({ sessionId: options.sessionId, ...params }),
      signal: AbortSignal.timeout(HOST_TIMEOUT_MS)
    });
    const text2 = await response.text();
    if (response.status === 403) throw new PermissionDeniedError(method, text2 || "not granted");
    if (!response.ok) throw new Error(`The host answered ${method} with HTTP ${response.status}`);
    if (text2 === "") return void 0;
    let parsed;
    try {
      parsed = JSON.parse(text2);
    } catch {
      throw new HostReplyError(method, "a body that is not JSON");
    }
    if (!parsed || typeof parsed !== "object" || !("result" in parsed)) {
      throw new HostReplyError(method, "a body carrying no result");
    }
    return parsed.result;
  }
  return {
    diff: () => ask("diff", {}),
    status: () => ask("status", {}),
    output: (opts) => ask("output", { ...opts?.lines !== void 0 && { lines: opts.lines } }),
    selection: () => ask("selection", {}),
    send: (text2) => ask("send", { text: text2 }),
    rename: (name) => ask("rename", { name }),
    usage: () => ask("usage", {})
  };
}
var RESERVED_KEYS = [
  "externalId",
  "title",
  "url",
  "description",
  "status",
  "labels",
  "assignee",
  "updatedAt"
];
var UNSAFE_KEYS2 = ["__proto__", "constructor", "prototype"];
function itemExternalId(item) {
  return String(item.externalId ?? "").trim();
}
function itemTimestamp(item, fallback) {
  return isoTimestamp(item.updatedAt, fallback);
}
function isoTimestamp(value2, fallback) {
  if (value2 === void 0) return fallback;
  const date = value2 instanceof Date ? value2 : new Date(value2);
  if (Number.isNaN(date.getTime())) {
    throw new Error(`Invalid updatedAt: ${String(value2)}`);
  }
  return date.toISOString();
}
function normalizeItem(item, polledAt) {
  const externalId = itemExternalId(item);
  if (!externalId) {
    throw new Error("Connector item is missing externalId");
  }
  if (!item.title || !item.title.trim()) {
    throw new Error(`Connector item ${externalId} is missing title`);
  }
  const extra = {};
  for (const [key2, value2] of Object.entries(item.data ?? {})) {
    if (RESERVED_KEYS.includes(key2)) continue;
    if (UNSAFE_KEYS2.includes(key2)) continue;
    extra[key2] = value2;
  }
  return {
    ...extra,
    externalId,
    title: item.title,
    url: item.url ?? "",
    description: item.description ?? "",
    status: item.status ?? "open",
    labels: item.labels ?? [],
    ...item.assignee !== void 0 && { assignee: item.assignee },
    updatedAt: isoTimestamp(item.updatedAt, polledAt)
  };
}
function normalizeItems(items, polledAt) {
  const seen = /* @__PURE__ */ new Set();
  return items.map((item) => {
    const normalized = normalizeItem(item, polledAt);
    if (seen.has(normalized.externalId)) {
      throw new Error(`Duplicate externalId "${normalized.externalId}" in one poll page`);
    }
    seen.add(normalized.externalId);
    return normalized;
  });
}
var MAX_BOUNDARY_IDS = 500;
function decodeCursor(cursor, strategy) {
  if (!cursor) return void 0;
  let parsed;
  try {
    parsed = JSON.parse(cursor);
  } catch (error) {
    throw new Error(`Cursor is not valid SDK cursor JSON: ${cursor}`, { cause: error });
  }
  const state = parsed;
  if (!state || typeof state !== "object" || state.v !== 1 || state.s !== strategy) {
    throw new Error(`Cursor does not belong to the "${strategy}" strategy: ${cursor}`);
  }
  const wellFormed = state.s === "timestamp" ? typeof state.t === "string" && Array.isArray(state.ids) && state.ids.every((id) => typeof id === "string") : typeof state.id === "string";
  if (!wellFormed) {
    throw new Error(`Cursor is missing the fields the "${strategy}" strategy needs: ${cursor}`);
  }
  return state;
}
function compare(left, right) {
  if (left === right) return 0;
  return left < right ? -1 : 1;
}
function page(chronological, context, hadCursor, nextCursor) {
  const delivered = context.limit === void 0 ? chronological : chronological.slice(0, context.limit);
  if (delivered.length === 0) {
    return { items: [], ...context.cursor !== void 0 && { nextCursor: context.cursor } };
  }
  return {
    items: delivered.map((entry) => entry.item),
    nextCursor: JSON.stringify(nextCursor(delivered)),
    // Only drain a backlog we know we truncated, and only once a cursor
    // exists — a first poll should not pull the source's entire history.
    hasMore: chronological.length > delivered.length && hadCursor
  };
}
function timestampPoll(fetched, state, context, polledAt) {
  const boundary = state?.t ?? context.since;
  const seen = new Set(state?.ids ?? []);
  const fresh = [];
  const pinnedAlreadySeen = [];
  for (const item of fetched) {
    const id = itemExternalId(item);
    const pinned = item.updatedAt === void 0 && boundary !== void 0;
    const at = pinned ? boundary : itemTimestamp(item, polledAt);
    const isNew = boundary === void 0 || at > boundary || at === boundary && !seen.has(id);
    if (isNew) fresh.push({ item, at, id, ...pinned && { pinned: true } });
    else if (pinned) pinnedAlreadySeen.push(id);
  }
  fresh.sort((left, right) => compare(left.at, right.at) || compare(left.id, right.id));
  return page(fresh, context, state !== void 0, (delivered) => {
    const newest = delivered[delivered.length - 1].at;
    const atNewest = [];
    for (let i = delivered.length - 1; i >= 0 && delivered[i].at === newest; i -= 1) {
      atNewest.push(delivered[i].id);
    }
    const carried = newest === boundary ? [...seen, ...atNewest] : [
      ...pinnedAlreadySeen,
      ...delivered.filter((entry) => entry.pinned).map((entry) => entry.id),
      ...atNewest
    ];
    return { v: 1, s: "timestamp", t: newest, ids: carried.slice(-MAX_BOUNDARY_IDS) };
  });
}
function lastItemPoll(fetched, state, context, polledAt) {
  const keyed = fetched.map((item) => ({
    item,
    at: itemTimestamp(item, polledAt),
    id: itemExternalId(item)
  }));
  const stopAt = state ? keyed.findIndex((entry) => entry.id === state.id) : -1;
  const chronological = (stopAt === -1 ? keyed : keyed.slice(0, stopAt)).reverse();
  return page(chronological, context, state !== void 0, (delivered) => ({
    v: 1,
    s: "lastItem",
    id: delivered[delivered.length - 1].id
  }));
}
async function pollWithDedupe(trigger, context) {
  const strategy = trigger.dedupe;
  const fetchItems = trigger.fetch;
  if (!strategy || !fetchItems) {
    throw new Error(`Trigger ${trigger.type} is not a declarative trigger`);
  }
  const polledAt = context.now();
  if (strategy === "lastItem") {
    const state2 = decodeCursor(context.cursor, "lastItem");
    const fetched2 = await runFetch(trigger.type, fetchItems, {
      config: context.config,
      ...state2 && { lastItemId: state2.id },
      ...context.limit !== void 0 && { limit: context.limit },
      now: context.now,
      fetch: context.fetch,
      ...context.session && { session: context.session }
    });
    return lastItemPoll(fetched2, state2, context, polledAt);
  }
  const state = decodeCursor(context.cursor, "timestamp");
  const since = state?.t ?? context.since;
  const fetched = await runFetch(trigger.type, fetchItems, {
    config: context.config,
    ...since !== void 0 && { since },
    ...context.limit !== void 0 && { limit: context.limit },
    now: context.now,
    fetch: context.fetch,
    ...context.session && { session: context.session }
  });
  return timestampPoll(fetched, state, context, polledAt);
}
async function runFetch(type, fetchItems, context) {
  const fetched = await fetchItems(context);
  if (!Array.isArray(fetched)) {
    throw new Error(`Trigger ${type} fetch() did not return an array`);
  }
  return fetched;
}
var MAX_ERROR_BODY = 500;
var PLACEHOLDER = /\{\{\s*(args|config)\.([A-Za-z0-9_.-]+)\s*\}\}/g;
var WHOLE_PLACEHOLDER = /^\{\{\s*(args|config)\.([A-Za-z0-9_.-]+)\s*\}\}$/;
function lookup(source, path2, scope) {
  return valueAt(source === "args" ? scope.args : scope.config, path2);
}
var intoUrl = (value2, source) => source === "config" ? value2 : encodeURIComponent(value2);
function intoHeader(name) {
  return (value2) => {
    if (/[\r\n]/.test(value2)) {
      throw new Error(`Header "${name}" would carry a line ending, which is not allowed`);
    }
    return value2;
  };
}
function resolveTemplates(value2, scope, substitute) {
  if (typeof value2 === "string") {
    const whole = WHOLE_PLACEHOLDER.exec(value2);
    if (whole) {
      const resolved = lookup(whole[1], whole[2], scope);
      if (substitute === void 0 || resolved === void 0 || resolved === null) return resolved;
      return substitute(String(resolved), whole[1]);
    }
    return value2.replace(PLACEHOLDER, (_match, source, path2) => {
      const resolved = lookup(source, path2, scope);
      if (resolved === void 0 || resolved === null) return "";
      const text2 = String(resolved);
      return substitute === void 0 ? text2 : substitute(text2, source);
    });
  }
  if (Array.isArray(value2)) return value2.map((entry) => resolveTemplates(entry, scope, substitute));
  if (typeof value2 === "object" && value2 !== null) {
    const out = {};
    for (const [key2, entry] of Object.entries(value2)) {
      out[key2] = resolveTemplates(entry, scope, substitute);
    }
    return out;
  }
  return value2;
}
function resolveHeaders(raw, scope) {
  const out = {};
  for (const [name, value2] of Object.entries(raw ?? {})) {
    const resolved = resolveTemplates(value2, scope, intoHeader(name));
    if (resolved === void 0 || resolved === null || resolved === "") continue;
    out[name] = String(resolved);
  }
  return out;
}
function stringMap(raw) {
  const out = {};
  if (typeof raw !== "object" || raw === null) return out;
  for (const [key2, value2] of Object.entries(raw)) {
    if (value2 === void 0 || value2 === null || value2 === "") continue;
    out[key2] = String(value2);
  }
  return out;
}
function resolveRequest(request, scope) {
  const method = (request.method ?? "GET").toUpperCase();
  const rawUrl = resolveTemplates(request.url, scope, intoUrl);
  if (typeof rawUrl !== "string" || rawUrl.trim() === "") {
    throw new Error("Request has no URL once its templates are resolved");
  }
  let url;
  try {
    url = new URL(rawUrl);
  } catch {
    throw new Error(`Request URL is not a URL once its templates are resolved: "${rawUrl}"`);
  }
  for (const [key2, value2] of Object.entries(stringMap(resolveTemplates(request.query, scope)))) {
    url.searchParams.set(key2, value2);
  }
  const headers = resolveHeaders(request.headers, scope);
  const resolved = { url: url.toString(), method, headers };
  if (request.body !== void 0 && method !== "GET" && method !== "HEAD") {
    const body = resolveTemplates(request.body, scope);
    if (body !== void 0) {
      resolved.body = typeof body === "string" ? body : JSON.stringify(body);
      if (!Object.keys(headers).some((key2) => key2.toLowerCase() === "content-type")) {
        resolved.headers["content-type"] = typeof body === "string" ? "text/plain" : "application/json";
      }
    }
  }
  return resolved;
}
async function readBody(response) {
  const text2 = await response.text();
  if (text2 === "") return void 0;
  const type = response.headers.get("content-type") ?? "";
  if (!type.includes("json")) return text2;
  try {
    return JSON.parse(text2);
  } catch {
    return text2;
  }
}
function describeFailure(response, body) {
  const detail = typeof body === "string" ? body : body === void 0 ? "" : JSON.stringify(body);
  const quoted = detail.length > MAX_ERROR_BODY ? `${detail.slice(0, MAX_ERROR_BODY)}\u2026` : detail;
  return `Request failed with ${response.status} ${response.statusText}${quoted ? `: ${quoted}` : ""}`;
}
async function sendRequest(resolved, options) {
  const response = await options.fetchImpl(resolved.url, {
    method: resolved.method,
    headers: resolved.headers,
    ...resolved.body !== void 0 && { body: resolved.body }
  });
  const body = await readBody(response);
  if (!response.ok) {
    throw new UpstreamStatusError(
      response.status,
      describeFailure(response, body),
      options.viaSession
    );
  }
  return { response, body };
}
function asOutput(value2) {
  if (Array.isArray(value2)) return { items: value2 };
  if (typeof value2 === "object" && value2 !== null) return value2;
  return value2 === void 0 ? {} : { result: value2 };
}
var MAX_REQUEST_PAGES = 100;
var LINK_NEXT = /<([^>]+)>\s*;[^,]*\brel\s*=\s*"?next"?/i;
function nextLink(header) {
  const match = header === null ? null : LINK_NEXT.exec(header);
  return match ? match[1] : void 0;
}
function pageItems(body, itemsPath) {
  const value2 = itemsPath === void 0 ? body : valueAt(body, itemsPath);
  return Array.isArray(value2) ? value2 : void 0;
}
async function collectPages(request, strategy, scope, options) {
  const collected = [];
  const seen = /* @__PURE__ */ new Set();
  let page2 = strategy.kind === "page" ? strategy.startPage ?? 1 : 0;
  let cursor;
  let nextUrl;
  for (let index = 0; index < MAX_REQUEST_PAGES; index++) {
    const resolved = resolveRequest(request, scope);
    if (nextUrl !== void 0) resolved.url = nextUrl;
    if (strategy.kind === "cursor" && cursor !== void 0) {
      const url = new URL(resolved.url);
      url.searchParams.set(strategy.param, cursor);
      resolved.url = url.toString();
    }
    if (strategy.kind === "page") {
      const url = new URL(resolved.url);
      url.searchParams.set(strategy.param, String(page2));
      resolved.url = url.toString();
    }
    if (seen.has(resolved.url)) {
      throw new Error(`Request for ${request.url} asked for the same page twice`);
    }
    seen.add(resolved.url);
    const { response, body } = await sendRequest(resolved, options);
    const items = pageItems(body, strategy.itemsPath);
    if (items === void 0) {
      if (index === 0) {
        const where = strategy.itemsPath === void 0 ? "the response is not a list" : `the response has no list at "${strategy.itemsPath}"`;
        throw new Error(`Cannot page through this request: ${where}`);
      }
      return collected;
    }
    collected.push(...items);
    if (items.length === 0) return collected;
    if (strategy.kind === "cursor") {
      const next = valueAt(body, strategy.cursorPath);
      if (next === void 0 || next === null || next === "") return collected;
      cursor = String(next);
      continue;
    }
    if (strategy.kind === "link") {
      nextUrl = nextLink(response.headers.get("link"));
      if (nextUrl === void 0) return collected;
      continue;
    }
    page2 += 1;
  }
  throw new Error(`Request for ${request.url} exceeded ${MAX_REQUEST_PAGES} pages`);
}
async function executeRequest(request, postReceive, scope, options) {
  if (request.paginate) {
    const items = await collectPages(request, request.paginate, scope, options);
    return asOutput(applyPostReceive(items, postReceive));
  }
  const { body } = await sendRequest(resolveRequest(request, scope), options);
  return asOutput(applyPostReceive(body, postReceive));
}
function wrap(fetchImpl, options, retryable) {
  return resilientFetch({
    fetchImpl,
    retryable,
    ...options.retry !== void 0 && { retry: options.retry },
    ...options.sleep !== void 0 && { sleep: options.sleep }
  });
}
function sessionFor(connector2, options, retryable) {
  if (connector2.auth?.rung !== "browser") return void 0;
  const fetchImpl = options.sessionFetchImpl ?? createSessionFetch(options.sessionCall ? { call: options.sessionCall } : {});
  return { fetch: wrap(fetchImpl, options, retryable) };
}
async function runPoll(connector2, triggerType, options = {}) {
  const trigger = connector2.triggers.find((entry) => entry.type === triggerType);
  if (!trigger) {
    throw new UnknownNameError(`${connector2.id} has no trigger "${triggerType}"`);
  }
  const now = options.now ?? (() => (/* @__PURE__ */ new Date()).toISOString());
  const polledAt = now();
  const session = sessionFor(connector2, options, true);
  const context = {
    config: options.config ?? {},
    ...options.since !== void 0 && { since: options.since },
    ...options.cursor !== void 0 && { cursor: options.cursor },
    ...options.limit !== void 0 && { limit: options.limit },
    now,
    // A poll only reads, so every failure it meets is worth trying again.
    fetch: wrap(options.fetchImpl ?? globalThis.fetch, options, true),
    ...session && { session }
  };
  const outcome = typeof trigger.poll === "function" ? await trigger.poll(context) : await pollWithDedupe(trigger, context);
  if (!outcome || !Array.isArray(outcome.items)) {
    throw new Error(`Trigger ${triggerType} did not return an items array`);
  }
  if (outcome.hasMore && !outcome.nextCursor) {
    throw new Error(`Trigger ${triggerType} reported more pages without a nextCursor`);
  }
  return {
    items: normalizeItems(outcome.items, polledAt),
    ...outcome.nextCursor !== void 0 && { nextCursor: outcome.nextCursor },
    hasMore: outcome.hasMore === true
  };
}
var SAFE_METHODS = /* @__PURE__ */ new Set(["GET", "HEAD"]);
async function runOptions(connector2, name, options = {}) {
  const loader = connector2.options?.[name];
  if (!loader) {
    throw new UnknownNameError(`${connector2.id} serves no options set "${name}"`);
  }
  const session = sessionFor(connector2, options, true);
  const loaded = await loader({
    config: options.config ?? {},
    now: options.now ?? (() => (/* @__PURE__ */ new Date()).toISOString()),
    fetch: wrap(options.fetchImpl ?? globalThis.fetch, options, true),
    ...session && { session }
  });
  if (!Array.isArray(loaded)) {
    throw new Error(`Options set "${name}" did not return an array`);
  }
  return loaded.map(
    (entry) => typeof entry === "string" ? { value: entry } : { ...entry, value: String(entry.value) }
  );
}
var MAX_QUOTED_VALUE = 80;
function quote(value2) {
  return value2.length > MAX_QUOTED_VALUE ? `${value2.slice(0, MAX_QUOTED_VALUE)}\u2026` : value2;
}
var shown = (value2) => typeof value2 === "string" ? `"${quote(value2)}"` : quote(JSON.stringify(value2) ?? String(value2));
function coerceArg(value2, type) {
  if (type === "number") {
    if (typeof value2 === "number" && Number.isFinite(value2)) return value2;
    const parsed = typeof value2 === "string" ? Number(value2) : Number.NaN;
    if (Number.isNaN(parsed)) throw new Error(`Expected a number, got ${shown(value2)}`);
    return parsed;
  }
  if (type === "boolean") {
    if (typeof value2 === "boolean") return value2;
    if (value2 === "true") return true;
    if (value2 === "false") return false;
    throw new Error(`Expected a boolean, got ${shown(value2)}`);
  }
  if (type === "json") {
    if (typeof value2 !== "string") return value2;
    try {
      return JSON.parse(value2);
    } catch {
      throw new Error(`Expected JSON, got ${shown(value2)}`);
    }
  }
  if (typeof value2 === "string") return value2;
  return typeof value2 === "object" ? JSON.stringify(value2) : String(value2);
}
async function runAction(connector2, actionType, args, options = {}) {
  const action = connector2.actions.find((entry) => entry.type === actionType);
  if (!action) {
    throw new UnknownNameError(`${connector2.id} has no action "${actionType}"`);
  }
  const coerced = { ...args };
  for (const input of action.inputs ?? []) {
    const value2 = coerced[input.key];
    if (value2 === void 0 || value2 === null || value2 === "") {
      if (input.required) {
        throw new ActionArgumentError(input.key, `Action ${actionType} requires "${input.key}"`);
      }
      delete coerced[input.key];
      continue;
    }
    try {
      coerced[input.key] = coerceArg(value2, input.type);
    } catch (error) {
      throw new ActionArgumentError(
        input.key,
        `Action ${actionType} argument "${input.key}": ${error instanceof Error ? error.message : String(error)}`
      );
    }
  }
  const config = options.config ?? {};
  const method = (action.request?.method ?? "GET").toUpperCase();
  const retryable = action.idempotent === true || action.request !== void 0 && SAFE_METHODS.has(method);
  const fetchImpl = wrap(options.fetchImpl ?? globalThis.fetch, options, retryable);
  const session = sessionFor(connector2, options, retryable);
  if (action.request !== void 0) {
    try {
      return await executeRequest(
        action.request,
        action.postReceive,
        { args: coerced, config },
        { fetchImpl: session?.fetch ?? fetchImpl, viaSession: session !== void 0 }
      );
    } catch (error) {
      throw new Error(
        `Action ${actionType}: ${error instanceof Error ? error.message : String(error)}`,
        { cause: error }
      );
    }
  }
  if (typeof action.run !== "function") {
    throw new Error(`Action ${actionType} has neither a run() implementation nor a request`);
  }
  const output = await action.run(coerced, {
    config,
    now: options.now ?? (() => (/* @__PURE__ */ new Date()).toISOString()),
    fetch: fetchImpl,
    ...session && { session }
  });
  return output ?? {};
}
var CHECK_SESSION = {
  sessionId: "check",
  worktreePath: process.cwd(),
  agent: "claude"
};
var SDK_VERSION = true ? "0.8.0-beta.12" : "0.0.0-source";
var InvalidParams = class extends Error {
};
function text(params, key2) {
  const value2 = params[key2];
  if (typeof value2 !== "string" || value2 === "") {
    throw new InvalidParams(`"${key2}" must be a non-empty string`);
  }
  return value2;
}
function optionalText(params, key2) {
  const value2 = params[key2];
  if (value2 === void 0 || value2 === null) return void 0;
  if (typeof value2 !== "string") throw new InvalidParams(`"${key2}" must be a string`);
  return value2;
}
function optionalNumber(params, key2) {
  const value2 = params[key2];
  if (value2 === void 0 || value2 === null) return void 0;
  if (typeof value2 !== "number" || !Number.isFinite(value2)) {
    throw new InvalidParams(`"${key2}" must be a number`);
  }
  return value2;
}
function failure(id, error) {
  return { jsonrpc: "2.0", id, error };
}
function refusal2(id, code, message) {
  return failure(id, { code, message });
}
function createConnectorServer(connector2, options = {}) {
  let cached = options.config;
  const config = () => cached ??= resolveConfig(connector2);
  const now = options.now;
  let greeted = false;
  const runtime = (params) => ({
    config: config(),
    now,
    sessionCall: optionalText(params, "sessionCall")
  });
  const hostFor = (sessionId) => options.host?.(sessionId) ?? createExtensionHost({ sessionId });
  const sessionContext = (params) => {
    const agent = text(params, "agent");
    if (!EXTENSION_AGENTS.includes(agent)) {
      throw new InvalidParams(`"agent" must be one of ${EXTENSION_AGENTS.join(", ")}`);
    }
    const sessionId = text(params, "sessionId");
    return {
      sessionId,
      worktreePath: text(params, "worktreePath"),
      agent,
      host: hostFor(sessionId),
      now: now ?? (() => (/* @__PURE__ */ new Date()).toISOString())
    };
  };
  const methods = {
    [PROTOCOL_METHODS.manifest]: () => connectorManifest(connector2),
    // A throw is the connector saying "broken", which must not read as a passing check.
    [PROTOCOL_METHODS.preflight]: async () => {
      if (!connector2.preflight) return { ok: null };
      try {
        return { ...await connector2.preflight() };
      } catch (error) {
        return { ok: false, message: messageOf(error) };
      }
    },
    [PROTOCOL_METHODS.options]: async (params) => ({
      options: await runOptions(connector2, text(params, "name"), runtime(params))
    }),
    [PROTOCOL_METHODS.poll]: (params) => runPoll(connector2, text(params, "trigger"), {
      ...runtime(params),
      cursor: optionalText(params, "cursor"),
      since: optionalText(params, "since"),
      limit: optionalNumber(params, "limit")
    }),
    [PROTOCOL_METHODS.action]: (params) => {
      const action = text(params, "action");
      const args = params.args ?? {};
      if (!isRecord(args)) throw new InvalidParams('"args" must be an object');
      return runAction(connector2, action, args, runtime(params));
    },
    [PROTOCOL_METHODS.footer]: async (params) => {
      const id = text(params, "footer");
      const footer = connector2.contributes?.footers?.find((entry) => entry.id === id);
      if (!footer) throw new InvalidParams(`${connector2.id} contributes no footer "${id}"`);
      return { items: await footer.run(sessionContext(params)) };
    },
    [PROTOCOL_METHODS.handler]: async (params) => {
      const id = text(params, "handler");
      const handler = connector2.contributes?.linkHandlers?.find((entry) => entry.id === id);
      if (!handler) throw new InvalidParams(`${connector2.id} contributes no link handler "${id}"`);
      const context = { ...sessionContext(params), url: text(params, "url") };
      return { ...await handler.run(context) ?? {} };
    }
  };
  const hello = (id, params) => {
    const offered = params.protocols;
    if (!Array.isArray(offered)) {
      return refusal2(id, PROTOCOL_ERROR_CODES.invalidParams, '"protocols" must be a list');
    }
    if (!offered.includes(PROTOCOL_VERSION)) {
      return refusal2(
        id,
        PROTOCOL_ERROR_CODES.unsupportedProtocol,
        `Vorn offers connector protocol ${offered.join(", ") || "none"}; ${connector2.id} speaks ${PROTOCOL_VERSION}`
      );
    }
    greeted = true;
    const result = {
      protocol: PROTOCOL_VERSION,
      sdk: { name: "@vornrun/connector-sdk", version: SDK_VERSION },
      connector: { id: connector2.id, version: connector2.version, kind: connector2.kind }
    };
    return { jsonrpc: "2.0", id, result };
  };
  return {
    async handle(message) {
      if (!isRecord(message) || typeof message.id !== "number") return void 0;
      const { id, method } = message;
      if (typeof method !== "string") {
        return refusal2(id, PROTOCOL_ERROR_CODES.invalidParams, "A request names its method");
      }
      const params = message.params ?? {};
      if (!isRecord(params)) {
        return refusal2(id, PROTOCOL_ERROR_CODES.invalidParams, '"params" must be an object');
      }
      if (method === PROTOCOL_METHODS.hello) return hello(id, params);
      const run = Object.hasOwn(methods, method) ? methods[method] : void 0;
      if (!run) return refusal2(id, PROTOCOL_ERROR_CODES.methodNotFound, "Method not found");
      if (!greeted) {
        return refusal2(
          id,
          PROTOCOL_ERROR_CODES.beforeHello,
          `Call ${PROTOCOL_METHODS.hello} before ${method}`
        );
      }
      try {
        return { jsonrpc: "2.0", id, result: await run(params) };
      } catch (error) {
        if (error instanceof InvalidParams || error instanceof UnknownNameError) {
          return refusal2(id, PROTOCOL_ERROR_CODES.invalidParams, error.message);
        }
        return failure(id, protocolError(error));
      }
    }
  };
}
function frame(response) {
  try {
    const line = JSON.stringify(response);
    if (Buffer.byteLength(line) > MAX_FRAME_BYTES) {
      throw new Error(`The answer is over ${MAX_FRAME_BYTES} bytes`);
    }
    return line;
  } catch (error) {
    return JSON.stringify(failure(response.id, protocolError(error)));
  }
}
var SERVING = /* @__PURE__ */ Symbol.for("@vornrun/connector-sdk/serving");
async function serveConnector(connector2, options = {}) {
  const shared = globalThis;
  if (shared[SERVING]) return;
  shared[SERVING] = true;
  const reply2 = process.stdout.write.bind(process.stdout);
  process.stdout.write = process.stderr.write.bind(process.stderr);
  const server = createConnectorServer(connector2, options);
  let inFlight = 0;
  let ended = false;
  const finish2 = () => {
    if (ended && inFlight === 0) reply2("", () => process.exit(0));
  };
  const onLine = (line) => {
    if (line.trim() === "") return;
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      process.stderr.write(`${connector2.id}: skipped a line that is not JSON
`);
      return;
    }
    inFlight++;
    void server.handle(message).then((response) => {
      if (response) reply2(`${frame(response)}
`);
    }).finally(() => {
      inFlight--;
      finish2();
    });
  };
  process.stdin.on(
    "data",
    lineReader(onLine, () => {
      process.stderr.write(`${connector2.id}: a message was over ${MAX_FRAME_BYTES} bytes
`);
      process.exit(1);
    })
  );
  process.stdin.on("end", () => {
    ended = true;
    finish2();
  });
}

// rss/dist/index.js
var dist_exports = {};
__export(dist_exports, {
  connector: () => connector,
  default: () => index_default
});
import { createHash } from "crypto";
import { createHash as createHash2 } from "crypto";
import { mkdir, rename as rename2, rm, writeFile } from "fs/promises";
import os from "os";
import path from "path";
import { realpathSync } from "fs";
import { fileURLToPath } from "url";
var LATIN1 = "Agrave Aacute Acirc Atilde Auml Aring AElig Ccedil Egrave Eacute Ecirc Euml Igrave Iacute Icirc Iuml ETH Ntilde Ograve Oacute Ocirc Otilde Ouml times Oslash Ugrave Uacute Ucirc Uuml Yacute THORN szlig agrave aacute acirc atilde auml aring aelig ccedil egrave eacute ecirc euml igrave iacute icirc iuml eth ntilde ograve oacute ocirc otilde ouml divide oslash ugrave uacute ucirc uuml yacute thorn yuml";
var ENTITIES = {
  amp: "&",
  lt: "<",
  gt: ">",
  quot: '"',
  apos: "'",
  nbsp: "\xA0",
  rsquo: "\u2019",
  lsquo: "\u2018",
  rdquo: "\u201D",
  ldquo: "\u201C",
  hellip: "\u2026",
  mdash: "\u2014",
  ndash: "\u2013",
  copy: "\xA9",
  reg: "\xAE",
  trade: "\u2122",
  laquo: "\xAB",
  raquo: "\xBB",
  bull: "\u2022",
  middot: "\xB7",
  ...Object.fromEntries(LATIN1.split(" ").map((name, i) => [name, String.fromCodePoint(192 + i)]))
};
function codePoint(code) {
  const valid = Number.isInteger(code) && code > 0 && code <= 1114111 && (code < 55296 || code > 57343);
  return valid ? String.fromCodePoint(code) : void 0;
}
function decodeEntities(value) {
  return value.replace(/&(#[xX][0-9a-fA-F]+|#\d+|[A-Za-z][A-Za-z0-9]*);/g, (whole, name) => {
    if (name.startsWith("#")) {
      const hex = name[1] === "x" || name[1] === "X";
      return codePoint(hex ? parseInt(name.slice(2), 16) : parseInt(name.slice(1), 10)) ?? whole;
    }
    return ENTITIES[name] ?? ENTITIES[name.toLowerCase()] ?? whole;
  });
}
function tidy(value) {
  return value.split(/\r?\n|\r/).map((line) => line.replace(/[ \t ]+/g, " ").trim()).filter((line) => line !== "").join("\n");
}
function plainText(html) {
  return tidy(
    decodeEntities(
      html.replace(/<(script|style)\b[\s\S]*?<\/\1\s*>/gi, "").replace(/\s+/g, " ").replace(/<(br|hr)\b[^>]*>/gi, "\n").replace(/<\/?(p|h[1-6]|li|blockquote|pre|div|ul|ol|figure|figcaption|table|tr)\b[^>]*>/gi, "\n").replace(/<[^>]+>/g, "")
    )
  );
}
function inlineText(html) {
  return plainText(html).replace(/\n/g, " ");
}
function oneLine(value) {
  return tidy(value).replace(/\n/g, " ");
}
function escapeHtml(value) {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
var SUMMARY_LENGTH = 2e3;
function cut(value, max = SUMMARY_LENGTH) {
  const chars = Array.from(value);
  return chars.length <= max ? value : `${chars.slice(0, max - 1).join("")}\u2026`;
}
var NS = {
  rss1: "http://purl.org/rss/1.0/",
  rdf: "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
  content: "http://purl.org/rss/1.0/modules/content/",
  dc: "http://purl.org/dc/elements/1.1/",
  atom: "http://www.w3.org/2005/Atom",
  media: "http://search.yahoo.com/mrss/",
  xml: "http://www.w3.org/XML/1998/namespace"
};
var CONVENTIONAL = /* @__PURE__ */ new Map([
  ["rdf", NS.rdf],
  ["content", NS.content],
  ["dc", NS.dc],
  ["atom", NS.atom],
  ["media", NS.media],
  ["xml", NS.xml]
]);
var START_TAG = /<([A-Za-z_][\w.:-]*)((?:\s+[^\s=/>]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s"'>]+))?)*)\s*(\/?)>/y;
var END_TAG = /<\/\s*([^\s<>]+)\s*>/y;
var ATTRIBUTE = /([^\s=/>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+)))?/g;
function resolveUrl(href, base) {
  const trimmed = href.trim();
  if (trimmed === "") return "";
  try {
    return new URL(trimmed, base || void 0).href;
  } catch {
    return trimmed;
  }
}
function rebase(value, parent) {
  try {
    return new URL(value.trim(), parent || void 0).href;
  } catch {
    return parent;
  }
}
function qualify(name, scope, attribute) {
  const colon = name.indexOf(":");
  if (colon === -1) return { ns: attribute ? "" : scope.get("") ?? "", local: name };
  const prefix = name.slice(0, colon);
  return {
    ns: scope.get(prefix) ?? CONVENTIONAL.get(prefix) ?? `urn:undeclared:${prefix}`,
    local: name.slice(colon + 1)
  };
}
function after(source, close, from) {
  const at = source.indexOf(close, from);
  return at === -1 ? source.length : at + close.length;
}
function skipDeclaration(source, from) {
  let depth = 0;
  let quote2 = "";
  for (let j = from + 2; j < source.length; j++) {
    const c = source[j];
    if (quote2) {
      if (c === quote2) quote2 = "";
    } else if (source.startsWith("<!--", j)) {
      j = after(source, "-->", j + 4) - 1;
    } else if (c === '"' || c === "'") {
      quote2 = c;
    } else if (c === "[") {
      depth++;
    } else if (c === "]") {
      depth = Math.max(0, depth - 1);
    } else if (c === ">" && depth === 0) {
      return j + 1;
    }
  }
  return source.length;
}
function attributes(raw) {
  return [...raw.matchAll(ATTRIBUTE)].map((m) => [m[1], decodeEntities(m[2] ?? m[3] ?? m[4] ?? "")]);
}
function parseXml(source, baseUrl = "") {
  const stack = [];
  let root;
  let i = 0;
  const n = source.length;
  const addText = (text2, cdata) => {
    stack.at(-1)?.el.children.push({ text: text2, cdata });
  };
  const close = (name, at) => {
    const lower = name.toLowerCase();
    let k = stack.length - 1;
    while (k >= 0 && stack[k].el.name.toLowerCase() !== lower) k--;
    if (k >= 0) for (const open of stack.splice(k)) open.el.end = at;
  };
  while (i < n) {
    const lt = source.indexOf("<", i);
    const stop = lt === -1 ? n : lt;
    if (stop > i) addText(decodeEntities(source.slice(i, stop)), false);
    if (lt === -1) break;
    i = lt;
    if (source.startsWith("<!--", i)) {
      i = after(source, "-->", i + 4);
    } else if (source.startsWith("<![CDATA[", i)) {
      const end = source.indexOf("]]>", i + 9);
      addText(source.slice(i + 9, end === -1 ? n : end), true);
      i = end === -1 ? n : end + 3;
    } else if (source.startsWith("<?", i)) {
      i = after(source, "?>", i + 2);
    } else if (source.startsWith("<!", i)) {
      i = skipDeclaration(source, i);
    } else {
      const tag = source[i + 1] === "/" ? END_TAG : START_TAG;
      tag.lastIndex = i;
      const m = tag.exec(source);
      if (!m) {
        addText("<", false);
        i++;
        continue;
      }
      i = tag.lastIndex;
      if (tag === END_TAG) {
        close(m[1], lt);
        continue;
      }
      if (root !== void 0 && stack.length === 0) continue;
      const parent = stack.at(-1);
      const raw = attributes(m[2] ?? "");
      let scope = parent?.scope ?? /* @__PURE__ */ new Map();
      const declared = raw.filter(([name]) => name === "xmlns" || name.startsWith("xmlns:"));
      if (declared.length > 0) {
        scope = new Map(scope);
        for (const [name, value] of declared) scope.set(name === "xmlns" ? "" : name.slice(6), value);
      }
      const attrs = raw.filter(([name]) => name !== "xmlns" && !name.startsWith("xmlns:")).map(([name, value]) => ({ ...qualify(name, scope, true), value }));
      const parentBase = parent?.el.base ?? baseUrl;
      const xmlBase = attrs.find((a) => a.ns === NS.xml && a.local === "base");
      const el = {
        name: m[1],
        ...qualify(m[1], scope, false),
        attrs,
        children: [],
        base: xmlBase ? rebase(xmlBase.value, parentBase) : parentBase,
        start: i,
        end: i,
        truncated: false,
        source
      };
      if (parent) parent.el.children.push(el);
      else root = el;
      if (m[3] !== "/") stack.push({ el, scope });
    }
  }
  for (const open of stack) {
    open.el.end = n;
    open.el.truncated = true;
  }
  return root;
}
function isElement(node) {
  return "local" in node;
}
function elements(el, local, ns) {
  return (el?.children ?? []).filter((c) => isElement(c) && c.local === local && c.ns === ns);
}
function child(el, local, ns) {
  return elements(el, local, ns)[0];
}
function attr(el, local, ns = "") {
  return el?.attrs.find((a) => a.local === local && a.ns === ns)?.value ?? "";
}
function innerXml(el) {
  return el.source.slice(el.start, el.end).trim();
}
function textOf(el) {
  if (!el) return "";
  if (el.children.some(isElement)) return innerXml(el);
  return el.children.map((node) => {
    const { text: text2, cdata } = node;
    return cdata && text2.includes("&lt;") && !text2.includes("<") ? decodeEntities(text2) : text2;
  }).join("").trim();
}
var FEED_TYPES = {
  "application/rss+xml": "rss2",
  "application/atom+xml": "atom",
  "application/feed+json": "json",
  "application/json": "json"
};
var TYPE_OF_FORMAT = {
  rss2: "application/rss+xml",
  rss1: "application/rss+xml",
  atom: "application/atom+xml",
  json: "application/feed+json"
};
var ATTRIBUTE2 = /([^\s=/>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+)))?/g;
function attributesOf(tag) {
  const found = /* @__PURE__ */ new Map();
  for (const m of tag.matchAll(ATTRIBUTE2)) {
    const name = m[1].toLowerCase();
    if (!found.has(name)) found.set(name, decodeEntities(m[2] ?? m[3] ?? m[4] ?? "").trim());
  }
  return found;
}
function discoverFeeds(html, pageUrl) {
  const page2 = html.replace(/<!--[\s\S]*?-->/g, "");
  const baseHref = attributesOf(/<base\b([^>]*)>/i.exec(page2)?.[1] ?? "").get("href") ?? "";
  const base = resolveUrl(baseHref, pageUrl) || pageUrl;
  const links = [];
  for (const m of page2.matchAll(/<link\b([^>]*)>/gi)) {
    const attrs = attributesOf(m[1]);
    const rel = (attrs.get("rel") ?? "").toLowerCase().split(/\s+/);
    const type = (attrs.get("type") ?? "").split(";")[0].trim().toLowerCase();
    const href = attrs.get("href") ?? "";
    const format = FEED_TYPES[type];
    if (!rel.includes("alternate") || format === void 0 || href === "") continue;
    links.push({ url: resolveUrl(href, base), title: attrs.get("title") ?? "", type, format });
  }
  const preferred = links.some((link) => link.type === "application/feed+json");
  const seen = /* @__PURE__ */ new Set();
  return links.filter((link) => {
    if (preferred && link.type === "application/json") return false;
    if (seen.has(link.url)) return false;
    seen.add(link.url);
    return true;
  });
}
var MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
var ZONES = {
  UT: 0,
  GMT: 0,
  Z: 0,
  EST: -5,
  EDT: -4,
  CST: -6,
  CDT: -5,
  MST: -7,
  MDT: -6,
  PST: -8,
  PDT: -7
};
var RFC822 = /^(?:[A-Za-z]{2,},?\s*)?(\d{1,2})[\s-]+([A-Za-z]{3,})\.?[\s-]+(\d{4}|\d{2})\s+(\d{1,2}):(\d{2})(?::(\d{2}))?(?:\s*([A-Za-z]+|[+-]\d{4}|[+-]\d{2}:\d{2}))?/;
var RFC3339 = /^(\d{4})-(\d{2})-(\d{2})(?:[Tt\s](\d{2}):(\d{2})(?::(\d{2})(?:[.,](\d+))?)?\s*([Zz]|[+-]\d{2}:?\d{2})?)?$/;
function offsetMinutes(zone) {
  if (zone === void 0) return 0;
  const numeric = /^([+-])(\d{2}):?(\d{2})$/.exec(zone);
  if (numeric) return (numeric[1] === "-" ? -1 : 1) * (Number(numeric[2]) * 60 + Number(numeric[3]));
  const upper = zone.toUpperCase();
  if (upper in ZONES) return ZONES[upper] * 60;
  if (/^[A-IK-M]$/.test(upper)) return -(upper.charCodeAt(0) - 64 - (upper > "J" ? 1 : 0)) * 60;
  if (/^[N-Y]$/.test(upper)) return (upper.charCodeAt(0) - 77) * 60;
  return 0;
}
function utc(year, month, day, hour, minute, second, ms, offset) {
  if (month < 0 || month > 11 || day < 1 || day > 31 || hour > 23 || minute > 59 || second > 60) return "";
  if (new Date(Date.UTC(year, month, day)).getUTCMonth() !== month) return "";
  return new Date(Date.UTC(year, month, day, hour, minute, second, ms) - offset * 6e4).toISOString();
}
function fromRfc3339(value) {
  const m = RFC3339.exec(value);
  if (!m) return void 0;
  const ms = m[7] === void 0 ? 0 : Number(m[7].slice(0, 3).padEnd(3, "0"));
  const zone = m[8] === void 0 || /^z$/i.test(m[8]) ? void 0 : m[8];
  return utc(
    Number(m[1]),
    Number(m[2]) - 1,
    Number(m[3]),
    Number(m[4] ?? 0),
    Number(m[5] ?? 0),
    Number(m[6] ?? 0),
    ms,
    offsetMinutes(zone)
  );
}
function fromRfc822(value) {
  const m = RFC822.exec(value);
  if (!m) return void 0;
  const month = MONTHS.indexOf(m[2].slice(0, 3).toLowerCase());
  if (month === -1) return "";
  const short = m[3].length === 2 ? Number(m[3]) : void 0;
  const year = short === void 0 ? Number(m[3]) : short < 50 ? 2e3 + short : 1900 + short;
  return utc(year, month, Number(m[1]), Number(m[4]), Number(m[5]), Number(m[6] ?? 0), 0, offsetMinutes(m[7]));
}
function parseDate(value) {
  const trimmed = (value ?? "").trim();
  if (trimmed === "") return "";
  return fromRfc3339(trimmed) ?? fromRfc822(trimmed) ?? "";
}
function hashId(title, publishedAt) {
  return `sha256:${createHash("sha256").update(`${title}
${publishedAt}`).digest("hex").slice(0, 16)}`;
}
function authorName(value) {
  const text2 = inlineText(value);
  return /^\S+@\S+\s*\((.+)\)$/.exec(text2)?.[1]?.trim() ?? text2;
}
function unique(values) {
  return [...new Set(values.map(oneLine).filter((value) => value !== ""))];
}
function finish(draft, feedTitle, feedUrl) {
  return {
    ...draft,
    id: draft.id || draft.url || hashId(draft.title, draft.publishedAt),
    summary: cut(draft.summary),
    categories: unique(draft.categories),
    feedTitle,
    feedUrl
  };
}
function texts(el, local, ns) {
  return elements(el, local, ns).map(textOf);
}
function alternate(el, ns) {
  const links = elements(el, "link", ns).filter((link) => {
    const rel = attr(link, "rel").trim().toLowerCase() || "alternate";
    return rel === "alternate" || rel === "http://www.iana.org/assignments/relation/alternate";
  });
  const best = links.find((link) => attr(link, "type").toLowerCase().startsWith("text/html")) ?? links[0];
  return best ? resolveUrl(attr(best, "href"), best.base) : "";
}
function linkText(el) {
  return el ? resolveUrl(oneLine(textOf(el)), el.base) : "";
}
function rssItem(item, core) {
  const guid = child(item, "guid", core);
  const guidText = oneLine(textOf(guid));
  const permalink = attr(guid, "isPermaLink").trim().toLowerCase() !== "false" && /^https?:\/\//i.test(guidText) ? resolveUrl(guidText, guid.base) : "";
  const description = textOf(child(item, "description", core));
  const group = child(item, "group", NS.media);
  const media = textOf(child(item, "description", NS.media) ?? child(group, "description", NS.media));
  const encoded = textOf(child(item, "encoded", NS.content));
  return {
    id: guidText,
    title: inlineText(textOf(child(item, "title", core))) || inlineText(textOf(child(item, "title", NS.media) ?? child(group, "title", NS.media))),
    url: linkText(child(item, "link", core)) || permalink || alternate(item, NS.atom),
    author: authorName(textOf(child(item, "author", core))) || inlineText(textOf(child(item, "creator", NS.dc))),
    publishedAt: parseDate(textOf(child(item, "pubDate", core))) || parseDate(textOf(child(item, "date", NS.dc))),
    updatedAt: parseDate(textOf(child(item, "updated", NS.atom))),
    summary: plainText(description || media || encoded),
    html: encoded || description,
    categories: [...texts(item, "category", core), ...texts(item, "subject", NS.dc)].map(inlineText)
  };
}
function rss2(root, source) {
  const core = root.ns;
  const channel = child(root, "channel", core);
  const title = inlineText(textOf(child(channel, "title", core)));
  const items = [...elements(channel, "item", core), ...elements(root, "item", core)];
  return {
    feed: {
      title,
      url: source.finalUrl,
      siteUrl: linkText(child(channel, "link", core)) || (channel ? alternate(channel, NS.atom) : ""),
      format: "rss2"
    },
    items: items.filter((item) => !item.truncated).map((item) => finish(rssItem(item, core), title, source.feedUrl))
  };
}
function rss1(root, source) {
  const channel = root.children.find((node) => isElement(node) && node.local === "channel");
  const core = channel?.ns ?? NS.rss1;
  const title = inlineText(textOf(child(channel, "title", core)));
  const items = elements(root, "item", core).filter((item) => !item.truncated);
  return {
    feed: { title, url: source.finalUrl, siteUrl: linkText(child(channel, "link", core)), format: "rss1" },
    items: items.map((item) => {
      const description = textOf(child(item, "description", core) ?? child(item, "description", NS.dc));
      const encoded = textOf(child(item, "encoded", NS.content));
      const draft = {
        id: attr(item, "about", NS.rdf).trim(),
        title: inlineText(textOf(child(item, "title", core))),
        url: linkText(child(item, "link", core)),
        author: inlineText(textOf(child(item, "creator", NS.dc))),
        publishedAt: parseDate(textOf(child(item, "date", NS.dc))),
        updatedAt: "",
        summary: plainText(description || encoded),
        html: encoded || description,
        categories: texts(item, "subject", NS.dc).map(inlineText)
      };
      return finish(draft, title, source.feedUrl);
    })
  };
}
function atomText(el) {
  if (!el || attr(el, "src") !== "") return { text: "", html: "" };
  const type = attr(el, "type").trim().toLowerCase() || "text";
  if (type === "xhtml" || type === "application/xhtml+xml") {
    const div = el.children.find(isElement);
    const markup = div ? innerXml(div) : textOf(el);
    return { text: plainText(markup), html: markup };
  }
  const value = textOf(el);
  if (type === "html" || type === "text/html") return { text: plainText(value), html: value };
  return { text: tidy(value), html: escapeHtml(value) };
}
function atomAuthors(el, core) {
  return elements(el, "author", core).map((author) => oneLine(textOf(child(author, "name", core)))).filter((name) => name !== "").join(", ");
}
function atom(root, source) {
  const core = root.ns;
  const title = atomText(child(root, "title", core)).text.replace(/\n/g, " ");
  const feedAuthor = atomAuthors(root, core);
  const entries = elements(root, "entry", core).filter((entry) => !entry.truncated);
  return {
    feed: { title, url: source.finalUrl, siteUrl: alternate(root, core), format: "atom" },
    items: entries.map((entry) => {
      const summary = atomText(child(entry, "summary", core));
      const content = atomText(child(entry, "content", core));
      const updated = parseDate(textOf(child(entry, "updated", core)));
      const draft = {
        id: oneLine(textOf(child(entry, "id", core))),
        title: atomText(child(entry, "title", core)).text.replace(/\n/g, " "),
        url: alternate(entry, core),
        author: atomAuthors(entry, core) || feedAuthor,
        publishedAt: parseDate(textOf(child(entry, "published", core))) || updated,
        updatedAt: updated,
        summary: summary.text || content.text,
        html: content.html || summary.html,
        categories: elements(entry, "category", core).map((category) => attr(category, "term"))
      };
      return finish(draft, title, source.feedUrl);
    })
  };
}
function isRecord2(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function str(value) {
  return typeof value === "string" ? value.trim() : "";
}
function jsonAuthors(value) {
  const list = Array.isArray(value.authors) ? value.authors : [value.author];
  return list.filter(isRecord2).map((author) => oneLine(str(author.name))).filter((name) => name !== "").join(", ");
}
function jsonFeed(body, source) {
  let data;
  try {
    data = JSON.parse(body);
  } catch (error) {
    throw new Error(`unparseable: ${error.message}`);
  }
  const title = oneLine(str(data.title));
  const feedAuthor = jsonAuthors(data);
  const items = (Array.isArray(data.items) ? data.items : []).filter(isRecord2);
  return {
    feed: {
      title,
      url: source.finalUrl,
      siteUrl: resolveUrl(str(data.home_page_url), source.finalUrl),
      format: "json"
    },
    items: items.map((item) => {
      const html = str(item.content_html);
      const contentText = str(item.content_text);
      const draft = {
        id: typeof item.id === "string" || typeof item.id === "number" ? String(item.id).trim() : "",
        title: oneLine(str(item.title)),
        url: resolveUrl(str(item.url) || str(item.external_url), source.finalUrl),
        author: jsonAuthors(item) || feedAuthor,
        publishedAt: parseDate(str(item.date_published)),
        updatedAt: parseDate(str(item.date_modified)),
        summary: tidy(str(item.summary)) || tidy(contentText) || plainText(html),
        html: html || contentText && escapeHtml(contentText),
        categories: Array.isArray(item.tags) ? item.tags.filter((tag) => typeof tag === "string") : []
      };
      return finish(draft, title, source.feedUrl);
    })
  };
}
function isJsonFeed(body) {
  try {
    const data = JSON.parse(body);
    return isRecord2(data) && (typeof data.version === "string" || Array.isArray(data.items));
  } catch {
    return false;
  }
}
function parseFeed(body, source) {
  const text2 = body.replace(/^﻿/, "").trimStart();
  if (text2.startsWith("{")) return jsonFeed(text2, source);
  const root = parseXml(text2, source.finalUrl);
  if (!root) throw new Error("unparseable: no root element");
  const local = root.local.toLowerCase();
  if (local === "rss") return rss2(root, source);
  if (local === "rdf") return rss1(root, source);
  if (local === "feed") return atom(root, source);
  if (local === "html") {
    throw new Error(`not a feed (the body starts with "${/^[^\s>]{1,15}/.exec(text2)[0]}"); try find feeds`);
  }
  throw new Error(`unparseable: root <${root.name}> is not rss, RDF or feed`);
}
function itemTime(item) {
  return item.publishedAt || item.updatedAt;
}
function newestFirst(items) {
  return [...items].sort((a, b) => {
    const at = itemTime(a);
    const bt = itemTime(b);
    if (at === bt) return 0;
    if (at === "") return 1;
    if (bt === "") return -1;
    return at < bt ? 1 : -1;
  });
}
function withinHours(items, hours, nowMs) {
  const floor = nowMs - hours * 36e5;
  return items.filter((item) => itemTime(item) !== "" && Date.parse(itemTime(item)) >= floor);
}
var FEED_ACCEPT = "application/rss+xml, application/atom+xml, application/feed+json, application/xml;q=0.9, text/xml;q=0.8, */*;q=0.5";
var PAGE_ACCEPT = `text/html, application/xhtml+xml, ${FEED_ACCEPT}`;
var TIMEOUT_MS = 2e4;
var MAX_BYTES = 5 * 1024 * 1024;
var MAX_IN_FLIGHT = 6;
var FeedError = class extends Error {
  constructor(url, reason) {
    super(`${url}: ${reason}`);
    this.url = url;
    this.reason = reason;
    this.name = "FeedError";
  }
  url;
  reason;
};
var SCHEME = /^[a-z][a-z0-9+.-]*:/i;
var HOST_PORT = /^[^:/?#]+:\d+(?:[/?#]|$)/;
function feedAddress(raw) {
  const trimmed = raw.trim();
  const withScheme = SCHEME.test(trimmed) && !HOST_PORT.test(trimmed) ? trimmed : `https://${trimmed}`;
  let url;
  try {
    url = new URL(withScheme);
  } catch {
    throw new FeedError(trimmed, "not a valid address");
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new FeedError(trimmed, "refused, only http and https feeds are read");
  }
  return url.href;
}
function feedList(value) {
  const parts = String(value ?? "").split(/[\r\n,]+/).map((part) => part.trim());
  return [...new Set(parts.filter((part) => part !== ""))];
}
function uniqueFeeds(raws) {
  const urls = /* @__PURE__ */ new Set();
  const invalid = [];
  for (const raw of raws) {
    try {
      urls.add(feedAddress(raw));
    } catch (error) {
      invalid.push(error);
    }
  }
  return { urls: [...urls], invalid };
}
function discard(res) {
  res.body?.cancel().catch(() => void 0);
}
async function readCapped(res, url) {
  if (Number(res.headers.get("content-length")) > MAX_BYTES) {
    discard(res);
    throw new FeedError(url, "larger than 5 MB");
  }
  const chunks = [];
  let total = 0;
  const reader = res.body?.getReader();
  for (; ; ) {
    const next = await reader?.read();
    if (!next || next.done) break;
    total += next.value.byteLength;
    if (total > MAX_BYTES) {
      await reader.cancel().catch(() => void 0);
      throw new FeedError(url, "larger than 5 MB");
    }
    chunks.push(next.value);
  }
  const bytes = new Uint8Array(total);
  let at = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, at);
    at += chunk.byteLength;
  }
  return bytes;
}
function charset(bytes, contentType) {
  if (bytes[0] === 239 && bytes[1] === 187 && bytes[2] === 191) return "utf-8";
  if (bytes[0] === 255 && bytes[1] === 254) return "utf-16le";
  if (bytes[0] === 254 && bytes[1] === 255) return "utf-16be";
  const declared = /charset\s*=\s*["']?([^"';\s]+)/i.exec(contentType)?.[1];
  if (declared) return declared;
  const head = new TextDecoder("latin1").decode(bytes.subarray(0, 1024));
  return /^\s*<\?xml[^>]*?\bencoding\s*=\s*["']([^"']+)["']/.exec(head)?.[1] ?? "utf-8";
}
function decoderFor(label) {
  try {
    return new TextDecoder(label);
  } catch {
    return new TextDecoder("utf-8");
  }
}
function decodeBody(bytes, contentType) {
  return decoderFor(charset(bytes, contentType)).decode(bytes).replace(/^﻿/, "");
}
function reasonOf(error, timeoutMs) {
  if (!(error instanceof Error)) return String(error);
  if (error.name === "TimeoutError" || error.name === "AbortError") return `timed out after ${timeoutMs / 1e3} s`;
  const cause = error.cause instanceof Error ? error.cause.message : "";
  return cause && cause !== error.message ? `${error.message} (${cause})` : error.message;
}
async function send(fetchImpl, url, options) {
  const timeoutMs = options.timeoutMs ?? TIMEOUT_MS;
  const conditional = Boolean(options.etag || options.lastModified);
  const headers = {
    accept: options.accept ?? FEED_ACCEPT,
    "user-agent": options.userAgent,
    ...options.etag && { "if-none-match": options.etag },
    ...options.lastModified && { "if-modified-since": options.lastModified }
  };
  try {
    const res = await fetchImpl(url, { headers, redirect: "follow", signal: AbortSignal.timeout(timeoutMs) });
    if (res.status === 304 && conditional) {
      discard(res);
      return void 0;
    }
    if (!res.ok) {
      discard(res);
      throw new FeedError(url, `HTTP ${res.status}`);
    }
    const bytes = await readCapped(res, url);
    return {
      body: decodeBody(bytes, res.headers.get("content-type") ?? ""),
      url: res.url || url,
      etag: res.headers.get("etag") ?? "",
      lastModified: res.headers.get("last-modified") ?? ""
    };
  } catch (error) {
    if (error instanceof FeedError) throw error;
    throw new FeedError(url, reasonOf(error, timeoutMs));
  }
}
async function fetchBody(fetchImpl, url, options) {
  return await send(fetchImpl, url, options);
}
function fetchIfChanged(fetchImpl, url, options) {
  return send(fetchImpl, url, options);
}
function parseFetched(url, got) {
  try {
    return parseFeed(got.body, { feedUrl: url, finalUrl: got.url });
  } catch (error) {
    throw new FeedError(url, error.message);
  }
}
async function readFeedAt(fetchImpl, raw, options) {
  const url = feedAddress(raw);
  return parseFetched(url, await fetchBody(fetchImpl, url, options));
}
async function mapLimit(values, limit, fn) {
  const results = new Array(values.length);
  let next = 0;
  const worker = async () => {
    while (next < values.length) {
      const index = next++;
      results[index] = await fn(values[index]);
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, values.length) }, worker));
  return results;
}
var package_default = {
  name: "@vornrun/connector-rss",
  version: "0.1.3",
  description: "Trigger workflows from new items in RSS, Atom and JSON Feed feeds, and read feeds or find the feeds a web page declares from a step.",
  type: "module",
  license: "MIT",
  repository: {
    type: "git",
    url: "git+https://github.com/vorn-run/extensions.git",
    directory: "rss"
  },
  keywords: [
    "vorn",
    "connector",
    "rss",
    "atom",
    "json-feed"
  ],
  bin: {
    "vorn-connector-rss": "dist/index.js"
  },
  main: "./dist/index.js",
  types: "./dist/index.d.ts",
  exports: {
    ".": {
      types: "./dist/index.d.ts",
      default: "./dist/index.js"
    }
  },
  files: [
    "dist",
    "README.md",
    "CHANGELOG.md"
  ],
  scripts: {
    build: "tsup",
    typecheck: "tsc --noEmit",
    test: "vitest run"
  },
  dependencies: {
    "@vornrun/connector-sdk": "^0.7.2"
  },
  devDependencies: {
    "@types/node": "^22.10.2",
    "@vitest/coverage-v8": "^4.1.10",
    tsup: "^8.5.1",
    typescript: "^6.0.3",
    vitest: "^4.1.10"
  },
  vorn: {
    publisher: "Vorn",
    category: "Social",
    keywords: [
      "rss",
      "atom",
      "json feed",
      "feeds",
      "news",
      "blogs",
      "syndication"
    ],
    auth: "Nothing to sign in to: feeds are public, so installing the connector is the whole setup.",
    hostsFrom: [
      "RSS_FEEDS"
    ]
  }
};
var DEFAULT_USER_AGENT = `vorn-connector-rss/${package_default.version} (+https://vorn.run)`;
var DEFAULT_LOOKBACK_HOURS = 24;
var MAX_LOOKBACK_HOURS = 8760;
function blank(value) {
  return value === void 0 || value === null || String(value).trim() === "";
}
function wholeNumber(value, name, min, max, fallback) {
  if (blank(value)) return fallback;
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < min || parsed > max) {
    throw new Error(`${name} must be a whole number from ${min}${max === Infinity ? " up" : ` to ${max}`}`);
  }
  return parsed;
}
function optionalHours(value, name) {
  if (blank(value)) return void 0;
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) throw new Error(`${name} must be a number of hours above 0`);
  return parsed;
}
function lookbackHours(config) {
  return wholeNumber(config.lookbackHours, "lookbackHours", 1, MAX_LOOKBACK_HOURS, DEFAULT_LOOKBACK_HOURS);
}
function userAgentOf(config) {
  return config.userAgent?.trim() || DEFAULT_USER_AGENT;
}
var SEEN_CAP = 1e3;
var HOUR_MS = 36e5;
function seenKey(id) {
  return createHash2("sha256").update(id).digest("hex").slice(0, 12);
}
function strings(value) {
  return Array.isArray(value) ? value.filter((entry) => typeof entry === "string") : [];
}
function readCursor(raw) {
  const states = /* @__PURE__ */ new Map();
  let parsed;
  try {
    parsed = JSON.parse(raw ?? "");
  } catch {
    return states;
  }
  if (parsed?.v !== 1 || typeof parsed.feeds !== "object" || parsed.feeds === null) return states;
  for (const [url, state] of Object.entries(parsed.feeds)) {
    if (typeof state !== "object" || state === null) continue;
    states.set(url, {
      t: typeof state.t === "string" ? state.t : "",
      ...typeof state.etag === "string" && { etag: state.etag },
      ...typeof state.lm === "string" && { lm: state.lm },
      seen: strings(state.seen)
    });
  }
  return states;
}
function writeCursor(states) {
  return JSON.stringify({ v: 1, feeds: Object.fromEntries(states) });
}
function selectNew(items, previous, floorMs) {
  const seen = new Set(previous?.seen);
  const t = previous?.t ?? "";
  const fired = /* @__PURE__ */ new Set();
  const fire = [];
  let newest = t;
  for (const item of items) {
    const at = itemTime(item);
    const key = seenKey(item.id);
    if (at > newest) newest = at;
    const fresh = previous === void 0 ? at !== "" && Date.parse(at) >= floorMs : at === "" ? !seen.has(key) : at > t || at === t && !seen.has(key);
    if (fresh && !fired.has(key)) {
      fired.add(key);
      fire.push(item);
    }
  }
  const keys = items.filter((item) => {
    const at = itemTime(item);
    return at === "" || at === newest;
  }).map((item) => seenKey(item.id));
  const carried = newest === t ? previous?.seen ?? [] : [];
  return { fire, state: { t: newest, seen: [.../* @__PURE__ */ new Set([...keys, ...carried])].slice(0, SEEN_CAP) } };
}
function oldestFirst(items) {
  return [...items].sort((a, b) => {
    const at = itemTime(a);
    const bt = itemTime(b);
    if (at === bt) return 0;
    if (at === "") return 1;
    if (bt === "") return -1;
    return at < bt ? -1 : 1;
  });
}
function toConnectorItem(item) {
  const at = itemTime(item);
  return {
    externalId: `${item.feedUrl} ${item.id}`,
    title: item.title || item.url || "Untitled item",
    ...item.url && { url: item.url },
    ...item.summary && { description: item.summary },
    ...item.author && { assignee: item.author },
    labels: item.categories,
    ...at && { updatedAt: at },
    data: { ...item }
  };
}
async function pollFeeds(ctx) {
  const raws = feedList(ctx.config.feeds);
  if (raws.length === 0) throw new Error("Add feed URLs to the connection's feeds");
  const hours = lookbackHours(ctx.config);
  const userAgent = userAgentOf(ctx.config);
  const previous = readCursor(ctx.cursor);
  const sinceMs = ctx.since === void 0 ? NaN : Date.parse(ctx.since);
  const floorMs = Number.isNaN(sinceMs) ? Date.parse(ctx.now()) - hours * HOUR_MS : sinceMs;
  const { urls, invalid } = uniqueFeeds(raws);
  const polled = await mapLimit(urls, MAX_IN_FLIGHT, async (url) => {
    const before = previous.get(url);
    try {
      const got = await fetchIfChanged(ctx.fetch, url, { userAgent, etag: before?.etag, lastModified: before?.lm });
      if (got === void 0) return { url, state: before, fire: [] };
      const { fire, state } = selectNew(parseFetched(url, got).items, before, floorMs);
      return {
        url,
        state: {
          t: state.t,
          ...got.etag && { etag: got.etag },
          ...got.lastModified && { lm: got.lastModified },
          seen: state.seen
        },
        fire
      };
    } catch (error) {
      return { url, state: before, fire: [], failure: error };
    }
  });
  const failures = [...invalid, ...polled.flatMap((result) => result.failure ? [result.failure] : [])];
  for (const failure2 of failures) process.stderr.write(`rss: ${failure2.url}: ${failure2.reason}
`);
  if (failures.length === urls.length + invalid.length) {
    throw new Error(`Every feed failed: ${failures.map((failure2) => failure2.message).join("; ")}`);
  }
  const next = /* @__PURE__ */ new Map();
  for (const result of polled) if (result.state) next.set(result.url, result.state);
  return {
    items: oldestFirst(polled.flatMap((result) => result.fire)).map(toConnectorItem),
    nextCursor: writeCursor(next),
    hasMore: false
  };
}
function filePath(value) {
  const raw = String(value ?? "").trim();
  if (raw === "") throw new ActionArgumentError("path", "path is required");
  if (raw.startsWith("~/")) return path.join(os.homedir(), raw.slice(2));
  if (raw.startsWith("~") || !path.isAbsolute(raw)) {
    throw new ActionArgumentError("path", "path must be an absolute path or start with ~/");
  }
  return path.resolve(raw);
}
async function writeJsonAtomically(target, value) {
  await mkdir(path.dirname(target), { recursive: true });
  const temporary = `${target}.${process.pid}.tmp`;
  try {
    await writeFile(temporary, `${JSON.stringify(value, null, 2)}
`);
    await rename2(temporary, target);
  } catch (error) {
    await rm(temporary, { force: true });
    throw error;
  }
}
var MAX_ITEMS = 100;
var DEFAULT_ITEMS = 20;
function keywordFilter(value) {
  const words = String(value ?? "").split(",").map((word) => word.trim()).filter((word) => word !== "");
  if (words.length === 0) return () => true;
  const patterns = words.map(
    (word) => new RegExp(`(?<![\\p{L}\\p{N}])${word.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&")}(?![\\p{L}\\p{N}])`, "iu")
  );
  return (item) => patterns.some((pattern) => pattern.test(item.title) || pattern.test(item.summary));
}
function dedupeByUrl(items) {
  const kept = /* @__PURE__ */ new Map();
  for (const item of items) {
    const key = item.url || `${item.feedUrl} ${item.id}`;
    const held = kept.get(key);
    if (!held || itemTime(item) > itemTime(held)) kept.set(key, item);
  }
  return [...kept.values()];
}
function asFeed(url, got) {
  if (got.body.trimStart().startsWith("{") && !isJsonFeed(got.body.trim())) return void 0;
  try {
    return parseFetched(url, got);
  } catch {
    return void 0;
  }
}
var SAMPLE_ITEM = {
  id: "tag:rssboard.org,2006:weblog.221",
  title: "How to Read an RSS Feed with Java Using XOM",
  url: "https://www.rssboard.org/news/221/read-rss-feed-java-using-xom",
  author: "Rogers Cadenhead",
  publishedAt: "2023-08-02T03:25:57.000Z",
  updatedAt: "",
  summary: "There are a lot of libraries for processing XML data with Java that can be used to read RSS feeds. One of the best is the open source library XOM created by the computer book author Elliotte Rusty Harold.",
  html: "<p>There are a lot of libraries for processing XML data with Java that can be used to read RSS feeds.</p>",
  categories: ["announcements,"],
  feedTitle: "RSS Advisory Board",
  feedUrl: "http://feeds.rssboard.org/rssboard"
};
var SINCE_HOURS_INPUT = {
  key: "sinceHours",
  label: "Within hours",
  type: "number",
  description: "Only items published within this many hours; items with no date are left out when set.",
  builderHint: "Match it to how often the workflow runs, such as 24 for a daily digest."
};
var ITEM_OUTPUT = {
  key: "items",
  description: "Items as { id, title, url, author, publishedAt, updatedAt, summary, html, categories, feedTitle, feedUrl }"
};
var READ_FEEDS_INPUTS = [
  {
    key: "urls",
    label: "Feed URLs",
    description: "Feed addresses, one per line or comma separated; the connection's feeds when empty.",
    builderHint: "Leave it empty to read the feeds the connection already polls."
  },
  SINCE_HOURS_INPUT,
  {
    key: "perFeed",
    label: "Items per feed",
    type: "number",
    description: `The newest items kept from each feed, a whole number from 1 to ${MAX_ITEMS}; ${DEFAULT_ITEMS} when empty.`
  },
  {
    key: "keywords",
    label: "Keywords",
    description: "Comma separated; an item passes when any one matches its title or summary as a whole word or phrase, ignoring case. Empty keeps everything.",
    builderHint: 'Whole words only: "AI" matches "AI agents" but not "said".'
  },
  {
    key: "minFeedsOk",
    label: "Feeds that must answer",
    type: "number",
    description: "The action fails, naming every failed feed, when fewer feeds than this answer; 1 when empty.",
    builderHint: "Set it to the number of feeds when every one of them has to be read."
  }
];
var FEEDS_ANSWERED_OUTPUTS = [
  { key: "count", type: "number", description: "How many items came back, in `items`" },
  { key: "feedsOk", type: "number", description: "How many feeds answered" },
  { key: "feedsFailed", description: "The feeds that did not answer, as [{ url, error }]" }
];
async function readMany(args, ctx) {
  const perFeed = wholeNumber(args.perFeed, "perFeed", 1, MAX_ITEMS, DEFAULT_ITEMS);
  const minFeedsOk = wholeNumber(args.minFeedsOk, "minFeedsOk", 0, Infinity, 1);
  const hours = optionalHours(args.sinceHours, "sinceHours");
  const matches = keywordFilter(args.keywords);
  const given = feedList(args.urls);
  const raws = given.length > 0 ? given : feedList(ctx.config.feeds);
  if (raws.length === 0) throw new Error("Give feed URLs in urls, or add them to the connection's feeds");
  const userAgent = userAgentOf(ctx.config);
  const nowMs = Date.parse(ctx.now());
  const { urls, invalid } = uniqueFeeds(raws);
  const read = await mapLimit(urls, MAX_IN_FLIGHT, async (url) => {
    try {
      return { items: (await readFeedAt(ctx.fetch, url, { userAgent })).items };
    } catch (error) {
      return { failure: error };
    }
  });
  const failures = [...invalid, ...read.flatMap((result) => result.failure ? [result.failure] : [])];
  const feedsFailed = failures.map((failure2) => ({ url: failure2.url, error: failure2.reason }));
  const total = urls.length + invalid.length;
  const feedsOk = total - failures.length;
  if (feedsOk < minFeedsOk) {
    throw new Error(
      `${feedsOk} of ${total} feeds answered, fewer than minFeedsOk ${minFeedsOk}: ${failures.map((f) => f.message).join("; ")}`
    );
  }
  const picked = read.flatMap(({ items: items2 = [] }) => {
    const recent = hours === void 0 ? items2 : withinHours(items2, hours, nowMs);
    return newestFirst(recent.filter(matches)).slice(0, perFeed);
  });
  const items = newestFirst(dedupeByUrl(picked));
  return { items, count: items.length, feedsOk, feedsFailed };
}
var connector = defineConnector({
  id: "rss",
  name: "RSS",
  version: package_default.version,
  description: "Trigger workflows from new items in RSS, Atom and JSON Feed feeds, and read feeds or find the feeds a web page declares from a step.",
  icon: {
    viewBox: "0 0 24 24",
    paths: [
      "M5.5 16a2.5 2.5 0 1 1 0 5a2.5 2.5 0 1 1 0-5z",
      "M3 13a8 8 0 0 1 8 8h3A11 11 0 0 0 3 10z",
      "M3 6a15 15 0 0 1 15 15h3A18 18 0 0 0 3 3z"
    ]
  },
  auth: { rung: "none" },
  config: [
    {
      key: "feeds",
      label: "Feeds",
      env: "RSS_FEEDS",
      description: "Feed addresses, one per line or comma separated. The new-item trigger polls them, and Read feeds uses them when it is given none.",
      builderHint: "RSS, Atom or JSON Feed addresses rather than the site's page; Find feeds turns a page into its feed addresses. https:// is assumed when there is no scheme."
    },
    {
      key: "lookbackHours",
      label: "First poll looks back (hours)",
      env: "RSS_LOOKBACK_HOURS",
      default: String(DEFAULT_LOOKBACK_HOURS),
      description: "How far back the first poll of a feed reaches, a whole number of hours from 1 to 8760, so a new connection does not fire for the whole backlog.",
      builderHint: "Only the first poll of each feed reads it; later polls fire on whatever is newer than the last one."
    },
    {
      key: "userAgent",
      label: "User-Agent",
      env: "RSS_USER_AGENT",
      default: DEFAULT_USER_AGENT,
      description: "Sent with every request; some hosts refuse requests without one.",
      builderHint: "Leave the default unless a host asks for a contact address in it."
    }
  ],
  triggers: [
    {
      type: "newItem",
      label: "New item",
      description: "An item appears in any of the connection's feeds. The first poll of a feed fires only for items inside the look-back, an item with no date fires once, and an item dated before the newest one already seen never fires.",
      poll: pollFeeds,
      sample: [toConnectorItem(SAMPLE_ITEM)]
    }
  ],
  actions: [
    {
      type: "readFeed",
      label: "Read a feed",
      description: `The newest items of one RSS, Atom or JSON Feed feed, up to ${MAX_ITEMS}, newest first with dateless items last.`,
      idempotent: true,
      inputs: [
        {
          key: "url",
          label: "Feed URL",
          required: true,
          description: "The feed's address; https:// is assumed when there is no scheme.",
          builderHint: "A feed, not a web page; run Find feeds on a page to get its feed."
        },
        {
          key: "limit",
          label: "Items",
          type: "number",
          description: `How many of the newest items, a whole number from 1 to ${MAX_ITEMS}; ${DEFAULT_ITEMS} when empty.`
        },
        SINCE_HOURS_INPUT
      ],
      outputs: [
        { key: "feed", description: "The feed as { title, url, siteUrl, format }, format one of rss2, rss1, atom, json" },
        ITEM_OUTPUT,
        { key: "count", type: "number", description: "How many items came back, in `items`" }
      ],
      sample: { url: "https://www.rssboard.org/files/sample-rss-2.xml" },
      run: async (args, ctx) => {
        const limit = wholeNumber(args.limit, "limit", 1, MAX_ITEMS, DEFAULT_ITEMS);
        const hours = optionalHours(args.sinceHours, "sinceHours");
        const { feed, items } = await readFeedAt(ctx.fetch, String(args.url ?? ""), {
          userAgent: userAgentOf(ctx.config)
        });
        const recent = hours === void 0 ? items : withinHours(items, hours, Date.parse(ctx.now()));
        const kept = newestFirst(recent).slice(0, limit);
        return { feed, items: kept, count: kept.length };
      }
    },
    {
      type: "readFeeds",
      label: "Read feeds",
      description: "Items from several feeds at once, filtered by age and keywords, one copy per address, newest first. A feed that fails is listed with its reason and never loses the others' items.",
      idempotent: true,
      inputs: READ_FEEDS_INPUTS,
      outputs: [ITEM_OUTPUT, ...FEEDS_ANSWERED_OUTPUTS],
      sample: {
        urls: "https://www.rssboard.org/files/sample-rss-2.xml\nhttps://github.com/nodejs/node/releases.atom"
      },
      run: (args, ctx) => readMany(args, ctx)
    },
    {
      type: "saveFeeds",
      label: "Save feeds to a file",
      description: "Read feeds exactly as Read feeds does, then write the items to a JSON file instead of returning them, so a large read never has to pass through a template.",
      // Writing the same file again with a fresh read is safe to repeat.
      idempotent: true,
      inputs: [
        {
          key: "path",
          label: "File",
          required: true,
          description: "Where to write, as an absolute path or one starting with ~/; its folder is created when missing.",
          builderHint: "For example ~/digests/items.json; a relative path is refused, and an existing file is replaced."
        },
        ...READ_FEEDS_INPUTS
      ],
      outputs: [
        { key: "path", type: "string", description: "The absolute path written, holding { generatedAt, feedsOk, feedsFailed, count, items }" },
        ...FEEDS_ANSWERED_OUTPUTS
      ],
      run: async (args, ctx) => {
        const target = filePath(args.path);
        const { items, count, feedsOk, feedsFailed } = await readMany(args, ctx);
        await writeJsonAtomically(target, { generatedAt: ctx.now(), feedsOk, feedsFailed, count, items });
        return { path: target, count, feedsOk, feedsFailed };
      }
    },
    {
      type: "findFeeds",
      label: "Find feeds",
      description: 'The RSS, Atom and JSON Feed feeds a web page declares in its link rel="alternate" tags, as absolute addresses with their titles; the address itself when it already is a feed.',
      idempotent: true,
      inputs: [
        {
          key: "url",
          label: "Page URL",
          required: true,
          description: "A web page's address, or a feed's; https:// is assumed when there is no scheme.",
          builderHint: "A site's home page usually declares its feeds; a page that declares none returns an empty list."
        }
      ],
      outputs: [
        { key: "feeds", description: "The feeds found, as [{ url, title, type, format }]" },
        { key: "count", type: "number", description: "How many feeds were found" },
        { key: "isFeed", type: "boolean", description: "Whether the address given is itself a feed" }
      ],
      sample: { url: "https://www.rssboard.org/" },
      run: async (args, ctx) => {
        const url = feedAddress(String(args.url ?? ""));
        const got = await fetchBody(ctx.fetch, url, { userAgent: userAgentOf(ctx.config), accept: PAGE_ACCEPT });
        const parsed = asFeed(url, got);
        if (parsed) {
          const { title, format } = parsed.feed;
          return { feeds: [{ url, title, type: TYPE_OF_FORMAT[format], format }], count: 1, isFeed: true };
        }
        const feeds = discoverFeeds(got.body, got.url);
        return { feeds, count: feeds.length, isFeed: false };
      }
    }
  ]
});
function isEntryPoint(moduleUrl, argv = process.argv) {
  const invoked = argv[1];
  if (invoked === void 0) return false;
  try {
    return realpathSync(fileURLToPath(moduleUrl)) === realpathSync(invoked);
  } catch {
    return false;
  }
}
async function serveIfEntryPoint(moduleUrl, serve = serveConnector) {
  if (!isEntryPoint(moduleUrl)) return false;
  await serve(connector);
  return true;
}
var index_default = connector;
await serveIfEntryPoint(import.meta.url);

// rss/vorn-connector-pack.js
import { readFileSync } from "node:fs";
var exported = Object.values(dist_exports).find((value) => value && Array.isArray(value.triggers));
if (!exported) throw new Error("./dist/index.js exports no connector");
if (process.argv.includes("--check")) {
  const manifest = connectorManifest(exported);
  let receipt;
  try {
    receipt = JSON.parse(readFileSync(new URL("./verified.json", import.meta.url), "utf8"));
  } catch {
  }
  const problems = selfCheck(manifest, receipt, RECORDED_CHECKS);
  for (const problem of problems) console.error(problem);
  if (problems.length === 0) console.log(`${manifest.id} ${manifest.version}: ${receipt.checks.join(", ")}`);
  process.exit(problems.length === 0 ? 0 : 1);
}
await serveConnector(exported);
