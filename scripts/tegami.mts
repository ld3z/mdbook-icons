import { tegami, type TegamiPlugin } from "tegami";
import { runCli } from "tegami/cli";
import { cargo } from "tegami/plugins/cargo";
import { github } from "tegami/plugins/github";

/**
 * crates.io answers 403 to unidentified clients under its data access policy, and
 * the cargo plugin's publish check calls `fetch` without a User-Agent (tegami 1.3.4),
 * which fails every publish. Remove once the plugin sends its own.
 */
const upstreamFetch = globalThis.fetch;
globalThis.fetch = (input, init) => {
  const url =
    typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
  if (!url.startsWith("https://crates.io/")) return upstreamFetch(input, init);

  const headers = new Headers(
    init?.headers ?? (input instanceof Request ? input.headers : undefined),
  );
  headers.set("User-Agent", "mdbook-icons release (https://github.com/ld3z/mdbook-icons)");
  return upstreamFetch(input, { ...init, headers });
};

/**
 * The git plugin tags releases as `{name}@{version}`, but the `binstall` metadata in
 * Cargo.toml resolves assets under `v{version}` and every existing release uses that
 * form. `enforce: "pre"` claims the tag before the git plugin applies its default.
 */
function vPrefixedTags(): TegamiPlugin {
  return {
    name: "v-prefixed-tags",
    enforce: "pre",
    initPublishPlan({ plan }) {
      for (const [id, packagePlan] of plan.packages) {
        const version = this.graph.get(id)?.version;
        if (!version) continue;

        (packagePlan.git ??= {}).tag = `v${version}`;
      }
    },
  };
}

const paper = tegami({
  // This package.json only exists to run Tegami; it is not a released package.
  ignore: ["npm:mdbook-icons-release"],
  plugins: [
    vPrefixedTags(),
    cargo(),
    github({
      repo: "ld3z/mdbook-icons",
      versionPr: {
        base: "main",
      },
    }),
  ],
});

await runCli(paper);
