import { h } from 'vue'
import DefaultTheme from 'vitepress/theme-without-fonts'
import Appearance from './Appearance.vue'
import '@fontsource-variable/geist'
import '@fontsource-variable/geist-mono'
import '@fontsource-variable/noto-sans-jp'
import './tokens.css'
import './style.css'

export default {
  extends: DefaultTheme,
  Layout: () => h(DefaultTheme.Layout, null, {
    'sidebar-nav-after': () => h(Appearance),
    'doc-before': () => h('div', { class: 'version-note' }, [
      'main ブランチのドキュメント · ',
      h('a', { href: '/installation' }, '公開版との違い'),
    ]),
  }),
}
