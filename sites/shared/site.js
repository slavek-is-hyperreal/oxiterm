// Tiny vanilla JS shared by the three sites. No tracking, no storage, no network except the demo iframe
// the visitor asks for. Everything works without it except the live demos.
document.querySelectorAll('[data-iframe-src]').forEach((btn) => {
  btn.addEventListener('click', () => {
    const poster = btn.closest('.demo-poster');
    const f = document.createElement('iframe');
    f.className = 'demo-frame';
    f.src = btn.getAttribute('data-iframe-src').replace(/&amp;/g, '&');
    f.title = btn.getAttribute('data-iframe-title') || 'OxiUI';
    f.loading = 'lazy';
    f.setAttribute('allow', '');
    poster.replaceWith(f);
    f.focus();
  });
});
