// A plain, modern Touying theme for the pico-gs2 slides: white content
// slides with the section as a small label over a bold title, dark title and
// section slides, Inter for text and JetBrains Mono for code (both
// installed in the devcontainer), and one accent colour. Derived from
// Touying's metropolis theme. Not a deck itself, so it lives outside the
// docs/slides/*.typ that `just slides` compiles.

#import "@preview/touying:0.8.0": *

// The palette.
#let ink = rgb("#0f172a") // text and the dark slides
#let muted = rgb("#64748b") // secondary text
#let rule = rgb("#e2e8f0") // table rules, the progress bar's track
#let wash = rgb("#f8fafc") // table header and alternate rows
#let accent = rgb("#4f46e5") // labels, emphasis, the progress bar
#let accent-soft = rgb("#a5b4fc") // the accent on the dark slides

#let sans = "Inter"
#let display = ("Inter Display", "Inter")
#let mono = "JetBrains Mono"

// A content slide: the section's name as a label, the slide's title, the
// body, and a footer with the deck's title, the slide number and a
// progress bar.
#let slide(
  title: auto,
  config: (:),
  repeat: auto,
  setting: body => body,
  composer: auto,
  ..bodies,
) = touying-slide-wrapper(self => {
  let header(self) = {
    set std.align(bottom)
    pad(x: 2.2em, bottom: 0.6em, {
      text(
        font: sans,
        size: 0.62em,
        weight: "semibold",
        fill: accent,
        tracking: 0.08em,
        upper(utils.display-current-heading(level: 1, numbered: false)),
      )
      v(0.3em, weak: true)
      text(
        font: display,
        size: 1.35em,
        weight: "bold",
        fill: ink,
        if title != auto { title } else {
          utils.display-current-heading(level: 2, numbered: false)
        },
      )
    })
  }
  let footer(self) = {
    set std.align(bottom)
    set text(size: 0.6em, fill: muted)
    pad(x: 2.2em, bottom: 0.9em, components.left-and-right(
      self.info.title,
      context utils.slide-counter.display() + " / " + utils.last-slide-number,
    ))
    place(bottom, components.progress-bar(height: 3pt, accent, rule))
  }
  let self = utils.merge-dicts(
    self,
    config-page(fill: white, header: header, footer: footer),
  )
  let new-setting = body => {
    show: std.align.with(horizon)
    show: setting
    body
  }
  touying-slide(
    self: self,
    config: config,
    repeat: repeat,
    setting: new-setting,
    composer: composer,
    ..bodies,
  )
})

// The title slide: the deck's title, subtitle and date on the dark ground.
#let title-slide(config: (:), ..args) = touying-slide-wrapper(self => {
  self = utils.merge-dicts(
    self,
    config-common(freeze-slide-counter: true),
    config-page(fill: ink, margin: (x: 3em, y: 2.5em)),
    config,
  )
  let info = self.info + args.named()
  let body = {
    set std.align(horizon)
    block(width: 100%, {
      text(font: display, size: 2.4em, weight: "bold", fill: white, info.title)
      v(0.4em)
      box(width: 3em, height: 4pt, fill: accent-soft)
      v(0.6em)
      if info.subtitle != none {
        text(size: 1.05em, fill: rgb("#cbd5e1"), info.subtitle)
      }
      if info.date != none {
        v(1.2em)
        text(size: 0.75em, fill: rgb("#94a3b8"), utils.display-info-date(self))
      }
    })
  }
  touying-slide(self: self, body)
})

// A section slide: its name on the dark ground, under the deck's short
// title.
#let new-section-slide(config: (:), level: 1, numbered: true, body) = touying-slide-wrapper(self => {
  self = utils.merge-dicts(
    self,
    config-common(freeze-slide-counter: true),
    config-page(fill: ink, margin: (x: 3em, y: 2.5em)),
    config,
  )
  let content = {
    set std.align(horizon)
    text(
      size: 0.8em,
      weight: "semibold",
      fill: accent-soft,
      tracking: 0.08em,
      upper(self.info.short-title),
    )
    v(0.3em)
    text(
      font: display,
      size: 2.2em,
      weight: "bold",
      fill: white,
      utils.display-current-heading(level: level, numbered: false),
    )
    v(0.5em)
    box(width: 3em, height: 4pt, fill: accent-soft)
    body
  }
  touying-slide(self: self, config: config, content)
})

// The theme: 16:9, the fonts, strong text in the accent, and tables with
// horizontal rules only, a tinted header row and banded rows.
#let pico-theme(..args, body) = {
  set text(font: sans, size: 16pt, fill: ink)
  // Typst sets raw text at 0.8em; this brings it to 0.92 of the text round it.
  show raw: set text(font: mono, size: 1.15em)
  show strong: set text(fill: accent, weight: "semibold")
  set list(marker: text(fill: accent, sym.bullet))
  set table(
    stroke: (x, y) => (
      top: if y == 0 { none } else { 0.6pt + rule },
      bottom: 0.6pt + rule,
    ),
    fill: (x, y) => if y == 0 { wash },
    inset: (x: 0.6em, y: 0.45em),
  )
  show table.cell.where(y: 0): set text(weight: "semibold", fill: ink)

  show: touying-slides.with(
    config-page(
      ..utils.page-args-from-aspect-ratio("16-9"),
      header-ascent: 0%,
      footer-descent: 0%,
      margin: (top: 4.4em, bottom: 2.4em, x: 2.2em),
    ),
    config-common(
      slide-fn: slide,
      new-section-slide-fn: new-section-slide,
    ),
    config-methods(alert: utils.alert-with-primary-color),
    config-colors(
      primary: accent,
      primary-light: accent-soft,
      secondary: ink,
      neutral-lightest: white,
      neutral-dark: ink,
      neutral-darkest: ink,
    ),
    ..args,
  )
  body
}
