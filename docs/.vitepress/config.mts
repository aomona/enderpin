import { defineConfig } from 'vitepress'

export default defineConfig({
  lang: 'ja',
  title: 'Enderpin',
  description: 'Minecraftのクライアントとサーバーを、同じ設定で準備・共有するためのドキュメント。',
  cleanUrls: true,
  appearance: { initialValue: 'auto' },
  lastUpdated: true,
  head: [['meta', { name: 'theme-color', content: '#ffffff' }]],
  themeConfig: {
    siteTitle: 'Enderpin',
    nav: [{ text: 'GitHub', link: 'https://github.com/aomona/enderpin' }],
    sidebar: [
      { text: 'はじめる', items: [
        { text: 'Enderpinについて', link: '/' },
        { text: 'インストール', link: '/installation' },
        { text: '初期セットアップ', link: '/setup' },
        { text: 'ログインと起動', link: '/launch' },
      ] },
      { text: '環境を管理する', items: [
        { text: 'MODを追加・更新する', link: '/packages' },
        { text: 'ファイル構成と共有設定', link: '/workspace' },
        { text: '別のPCと共有する', link: '/sharing' },
        { text: '権限とサンドボックス', link: '/sandbox' },
        { text: '旧形式から移行する', link: '/migration' },
      ] },
      { text: '調べる', items: [
        { text: 'コマンド一覧', link: '/commands' },
        { text: 'enderpin.toml の設定', link: '/configuration' },
        { text: '困ったときは', link: '/troubleshooting' },
        { text: '対応環境と制限', link: '/platforms' },
      ] },
      { text: '開発する', items: [
        { text: '開発と検証', link: '/contributing' },
        { text: '検証記録', link: '/verification' },
        { text: 'ライセンスと再利用', link: '/third-party' },
      ] },
    ],
    search: { provider: 'local', options: { miniSearch: { options: { tokenize: (text) => Array.from(new Intl.Segmenter('ja', { granularity: 'word' }).segment(text), part => part.isWordLike ? part.segment : '').filter(Boolean) } }, locales: { root: { translations: {
      button: { buttonText: 'ドキュメントを検索', buttonAriaLabel: 'ドキュメントを検索' },
      modal: { displayDetails: '本文を表示', resetButtonTitle: '検索をクリア', backButtonTitle: '検索を閉じる', noResultsText: '見つかりませんでした', footer: { selectText: '選択', navigateText: '移動', closeText: '閉じる' } },
    } } } } },
    outline: { label: 'このページ', level: [2, 3] },
    sidebarMenuLabel: '目次',
    returnToTopLabel: '先頭へ戻る',
    darkModeSwitchLabel: '表示テーマ',
    docFooter: { prev: '前のページ', next: '次のページ' },
    lastUpdated: { text: '最終更新', formatOptions: { dateStyle: 'short' } },
  },
})
