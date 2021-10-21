# StreamHead

A host-it-yourself livestreaming server

## What?

StreamHead is a small server that can ingest video streams and stream them through multiple transport protocols.

Currently it supports RTMP for ingest and fragmented MP4 over either websocket or directly through HTTP for consuming the streams.

## How?

StreamHead is mostly focused on the streaming backend, though there is some additional plumbing required for using eg. the websocket transport in a browser. There is a small `webapp` example folder which contains a website that streams video from a StreamHead server. See the *Testing* section below for instructions.

## Testing

After starting both the frontend and the backend using the following example the following addresses should be open:

* `localhost:8000` - Default address for python's `http.server` for the frontend
* `localhost:8080` - Exposes websocket endpoint for the frontend
* `localhost:1935` - Exposes RTMP ingest endpoint

### Frontend
```sh
cd webapp && python -m http.server
```

### Backend
```sh
cargo run
```

# License

StreamHead is licensed under the [Mozilla Public License](LICENSE-MPL).
