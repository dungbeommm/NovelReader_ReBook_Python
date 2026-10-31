/* Material 3 tonal schemes. Runs before stylesheets to avoid a mismatched first frame. */
(() => {
  'use strict';
  const {Hct, SchemeTonalSpot, SchemeMonochrome, MaterialDynamicColors, TonalPalette, argbFromHex, hexFromArgb} = MaterialColors;
  const presets = Object.freeze({monochrome: '#808080', rose: '#b3265b', ocean: '#0061a4', jade: '#006c4c', violet: '#6750a4', amber: '#8b5000'});
  const roles = ['primary', 'onPrimary', 'primaryContainer', 'onPrimaryContainer', 'secondary', 'onSecondary', 'secondaryContainer', 'onSecondaryContainer', 'tertiary', 'onTertiary', 'tertiaryContainer', 'onTertiaryContainer', 'error', 'onError', 'errorContainer', 'onErrorContainer', 'surface', 'surfaceDim', 'surfaceBright', 'surfaceContainerLowest', 'surfaceContainerLow', 'surfaceContainer', 'surfaceContainerHigh', 'surfaceContainerHighest', 'onSurface', 'onSurfaceVariant', 'outline', 'outlineVariant', 'inverseSurface', 'inverseOnSurface', 'inversePrimary'];
  const kebab = value => value.replace(/[A-Z]/g, letter => '-' + letter.toLowerCase());
  const favicon = document.querySelector('link[rel="icon"]');
  const faviconImage = new Image();
  let faviconColor = '#6750a4';
  function updateFavicon() {
    if (!favicon || !faviconImage.complete || !faviconImage.naturalWidth) return;
    const canvas = document.createElement('canvas');
    canvas.width = faviconImage.naturalWidth;
    canvas.height = faviconImage.naturalHeight;
    const context = canvas.getContext('2d', {willReadFrequently: true});
    context.drawImage(faviconImage, 0, 0);
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height);
    const red = parseInt(faviconColor.slice(1, 3), 16);
    const green = parseInt(faviconColor.slice(3, 5), 16);
    const blue = parseInt(faviconColor.slice(5, 7), 16);
    for (let index = 0; index < pixels.data.length; index += 4) {
      pixels.data[index] = red;
      pixels.data[index + 1] = green;
      pixels.data[index + 2] = blue;
    }
    context.putImageData(pixels, 0, 0);
    favicon.type = 'image/png';
    favicon.href = canvas.toDataURL('image/png');
  }
  faviconImage.onload = updateFavicon;
  faviconImage.src = '/static/logo.png?v=2';
  function apply(theme, choice) {
    const root = document.documentElement;
    const dark = theme !== 'light';
    const preset = Object.hasOwn(presets, choice);
    const custom = typeof choice === 'string' && /^#[\da-f]{6}$/i.test(choice);
    choice = preset || custom ? choice : 'violet';
    const seed = presets[choice] || choice;
    const source = Hct.fromInt(argbFromHex(seed));
    const scheme = choice === 'monochrome' ? new SchemeMonochrome(source, dark, 0) : new SchemeTonalSpot(source, dark, 0);
    faviconColor = hexFromArgb(MaterialDynamicColors.primary.getArgb(scheme));
    root.dataset.theme = dark ? 'dark' : 'light';
    root.dataset.accent = custom ? 'custom' : choice;
    root.style.setProperty('--user-accent', seed);
    for (const role of roles) root.style.setProperty('--md-sys-color-' + kebab(role), hexFromArgb(MaterialDynamicColors[role].getArgb(scheme)));
    // Additional semantic status colors follow the same HCT tone pairs as M3.
    for (const [name, hue] of [['success', 145], ['warning', 85]]) {
      const palette = TonalPalette.fromHueAndChroma(hue, 40);
      for (const [suffix, tone] of [['', dark ? 80 : 40], ['-container', dark ? 30 : 90], ['-on-container', dark ? 90 : 10]]) {
        root.style.setProperty('--md-status-' + name + suffix, hexFromArgb(palette.tone(tone)));
      }
    }
    updateFavicon();
    return {theme: dark ? 'dark' : 'light', choice, seed};
  }
  window.NovelTheme = Object.freeze({apply, presets});
  let theme = 'dark', accent = 'violet';
  try { theme = localStorage.getItem('novel_theme') || theme; accent = localStorage.getItem('novel_accent') || accent; } catch {}
  apply(theme, accent);
})();
