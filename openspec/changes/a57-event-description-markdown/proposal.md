# Change: Event descriptions render Markdown, like announcements already do

## Why

Announcements accept Markdown. Their editor says so — *"Supports Markdown
formatting (bold, italic, strikethrough, lists, headings, links, code,
blockquotes)"* — and `/public/announcements` carries a server-rendered,
server-sanitized `content_html` so consumers render formatted output without
running a parser or making a sanitization decision of their own.

Event descriptions got neither. The editor says nothing, and the description is
emitted as raw text everywhere it appears. Organizers write Markdown into it
regardless, because the announcement editor taught them the application accepts
it and nothing in the event editor says otherwise.

The result is visible in production right now. A recurring event's description
begins `**Monthly Hack the Box and Training Night**`, and those asterisks are
rendered literally: on the calendar modal of the marketing site, on the
Coterie-hosted registration page, and — most visibly — in the Open Graph
description of the event's share page, which is what a link to it previews as on
social media. Someone used the formatting the rest of the application taught them
to use, and it leaks as punctuation in a preview card.

This is not a rendering bug in any one of those surfaces. Each is faithfully
displaying the field it was given. The field is raw Markdown that nothing renders.

## What Changes

- Event descriptions are rendered by the **same** shared pipeline announcements
  already use — same safe subset, same scheme restriction, same refusal to emit
  inline images or raw HTML. Not a second renderer, and not a second set of
  sanitization decisions to keep aligned.
- Public event output carries a server-rendered sanitized description alongside
  the raw one, matching what `/public/announcements` already does, so a consumer
  renders formatted output without parsing or sanitizing anything itself.
- Coterie's own event surfaces render it rather than printing it as text.
- The event editor says Markdown is supported, in the same place and the same
  words the announcement editor does. This is the part that would have prevented
  the problem: the application accepted Markdown and never said so, and an
  organizer had no way to know whether it would be rendered.

## Why the editor hint matters as much as the rendering

An input that silently accepts a formatting language it does not render is worse
than one that rejects it, because the author gets no signal either way. They see
their asterisks in the box, the save succeeds, and the defect surfaces later on a
page they were not looking at. Announcements already carry the hint; events
carrying it too is what makes the two fields behave the same way rather than
merely look the same.

## What this does not do

- **It does not add a second renderer.** If the safe subset ever needs to change,
  it changes in one place, as it does today.
- **It does not touch stored data.** Existing descriptions are Markdown already;
  they simply begin rendering. Nothing is rewritten, and the raw text remains
  what the organizer typed.
- **It does not render Markdown in fields that are not descriptions.** Titles and
  locations stay plain text.
- **It does not change the marketing site.** theneontemple.com reads
  `description` and will keep rendering it as text until that repository is
  changed to prefer the rendered field — the same follow-on that
  `content_html` required for announcements. That belongs to that repository's
  specs, not this change.
