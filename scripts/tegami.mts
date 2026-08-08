import { tegami, type TegamiPlugin } from "tegami";
import { runCli } from "tegami/cli";
import { cargo } from "tegami/plugins/cargo";
import { github } from "tegami/plugins/github";

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
