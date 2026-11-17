# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

Rust desktop app for Linux and Windows. It draws its own UI (no native GTK or WinUI controls), so the design language is the same on both systems.

## Users

People who already use WoofMusic on iPhone and want the same library, history and feel at their computer.

## Product Purpose

A standalone YouTube Music player for the desk. It plays music by itself with the same Google account as WoofMusic iOS, so library, playlists, likes and YouTube history are the same on both devices. Success: a user moves from phone to desk without relearning anything or losing their library.

## Positioning

The desktop counterpart of WoofMusic: same data model, same features and same Liquid Glass identity as the iOS app, with logic translated file by file from WoofMusic and SwiftyTube so the two apps stay in step.

## Operating Context

- Runs on a desktop or laptop, usually in a wide window next to other work, controlled by mouse, keyboard and media keys.
- Long listening sessions in the background; the window is glanced at, not stared at.
- No ads, AutoMix and Smart Crossfade between songs, lyrics, offline downloads (features of WoofMusic iOS to be brought over).

## Capabilities and Constraints

- Plays on its own; does not need the iPhone to be on.
- Data comes from YouTube Music (InnerTube) through a Rust translation of SwiftyTube; playback through libmpv.
- Local-only data of the phone (play history, Recap, downloads) is not shared between devices.
- Login is Google sign-in in a browser window, harvesting the session cookies like WoofMusic iOS; cookie paste is the fallback.
- Undecided: controlling WoofMusic on the iPhone remotely.

## Brand Commitments

- Name: WoofMusic. App icons and assets come from the WoofMusic repository.
- Same visual identity as WoofMusic iOS 26: Liquid Glass controls, colours and icons. Layout adapts to a wide window used with a mouse (sidebar, larger lists) instead of copying phone screens.

## Evidence on Hand

- Screens of the iOS app: `.github/assets/1.png`, `2.png`, `3.png` in the WoofMusic repository (music content replaced by placeholders).
- No desktop screenshots, testimonials or user numbers exist yet.

## Product Principles

1. Same library, same feel: nothing the user knows from iOS should work differently without a desktop reason.
2. Stay in step with WoofMusic: logic is translated, not reinvented.
3. Music first: the window stays quiet so it can live next to other work.
4. Works the same on Linux and Windows.
