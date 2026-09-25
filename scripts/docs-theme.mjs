import { readFileSync, writeFileSync, copyFileSync } from 'node:fs'
const tokens = JSON.parse(readFileSync(new URL('../docs/.vitepress/design-tokens.json', import.meta.url)))
const variables = (values) => Object.entries(values).map(([key, value]) => `  --mona-${key.replaceAll('.', '-')}: ${value};`).join('\n')
const css = `/* Generated from MonaLauncher Design Tokens v1.1. Do not edit. */\n:root {\n${variables(tokens.color.light)}\n}\n.dark {\n${variables(tokens.color.dark)}\n}\n`
writeFileSync(new URL('../docs/.vitepress/theme/tokens.css', import.meta.url), css)
for (const name of ['geist', 'geist-mono', 'noto-sans-jp']) {
  copyFileSync(new URL(`../node_modules/@fontsource-variable/${name}/LICENSE`, import.meta.url), new URL(`../docs/public/fonts/licenses/${name}.txt`, import.meta.url))
}
