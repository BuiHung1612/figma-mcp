# 🗺️ Figma Rust MCP Roadmap

> Current release: **v5.0.0**. Shipped detail lives in [CHANGELOG.md](CHANGELOG.md); this file only tracks what is next.

---

## ✅ Shipped (v2.9 → v4.0)

- Semantic AST pruning and `clean-spec` output for `figma_to_code`
- Annotated screenshots (`withAnnotations`)
- Shadcn/UI & Radix component mapping (`framework="react-shadcn"`)
- Responsive breakpoint & flex inference
- Realtime selection streaming over WebSocket
- Scoped, viewport-first indexing with revisioned delta updates (v4.0.0)
- Local asset server (`/assets/*path`)

## 🧭 Next

- [ ] **Code-to-Figma live preview (`figma_preview_code`)** — render a generated React/HTML/Tailwind snippet into a temporary `[AI Preview]` frame for visual review before commit.
- [ ] **Design system scaffolder (`figma_scaffold_project`)** — one call writes `src/components/ui/*`, tokens, `tailwind.config.ts` and `globals.css` from the open file.
- [ ] **Multi-file design system sync** — cross-file libraries and shared variable collections across open tabs.

## 📏 Performance claims

Numbers in docs come from `npm run bench` or the plugin **Bench** tab, never estimates.
See the Benchmarks section of the [README](README.md#-benchmarks).

## 🏷️ Release policy

- Patch: fixes only, no tool schema or response shape changes.
- Minor: additive tools/fields; batch features into one minor instead of a release per change.
- Major: any breaking change to tool names, parameters or response contracts — listed under **Breaking** in the changelog.
- `--upgrade` stays on the installed major (`npm install -g figma-rust-mcp@4`); a new major is announced and needs an explicit `@latest` install.

---

*Maintained by [@BuiHung1612](https://github.com/BuiHung1612).*
