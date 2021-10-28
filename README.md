# StreamHead

A host-it-yourself livestreaming server

## What?

StreamHead is a small server that can ingest video streams and stream them through multiple transport protocols.

Currently it supports RTMP for ingest and fragmented MP4 over either websocket or directly through HTTP for consuming the streams.

## How?

StreamHead is mostly focused on the streaming backend, though there is some additional plumbing required for using eg. the websocket transport in a browser. There is a small `webapp` example folder which contains a website that streams video from a StreamHead server. See the *Testing* section below for instructions.

## Features

StreamHead supports a number of ingest and transport protocols:

### Ingest

* RTMP
* MPEG-TS over HTTP

### Transport

* Fragmented MP4 over WebSocket
* Fragmented MP4 over HTTP
* WebRTC (signalling over WebSocket)

# License

StreamHead is licensed under the [Mozilla Public License](LICENSE-MPL).
