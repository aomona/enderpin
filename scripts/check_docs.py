"""Check generated HTML links, fragments and local assets using only the stdlib."""
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path('docs/.vitepress/dist').resolve()

class Page(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.ids, self.links = set(), []
        self.feed(path.read_text())

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if 'id' in attrs:
            self.ids.add(attrs['id'])
        for key in ('href', 'src'):
            if attrs.get(key):
                self.links.append(attrs[key])

pages = {path: Page(path) for path in ROOT.rglob('*.html')}
assert pages, 'Build the documentation first'
errors = []
for source, page in pages.items():
    for link in page.links:
        url = urlsplit(link)
        if url.scheme or url.netloc:
            continue
        target = (ROOT / unquote(url.path).lstrip('/') if url.path.startswith('/')
                  else source.parent / unquote(url.path)) if url.path else source
        if target.is_dir():
            target /= 'index.html'
        elif not target.exists() and not target.suffix:
            target = target.with_suffix('.html')
        target = target.resolve()
        if not target.is_file():
            errors.append(f'{source.name}: missing {link}')
        elif url.fragment and target in pages and unquote(url.fragment) not in pages[target].ids:
            errors.append(f'{source.name}: missing fragment {link}')
assert not errors, '\n'.join(errors)
print(f'Validated links, fragments and assets in {len(pages)} HTML pages')
