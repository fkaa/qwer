# Work in progress!

# Testing

After starting both the frontend and the backend using the following example the following addresses should be open:

* `localhost:8000` - Default address for python's `http.server` for the frontend
* `localhost:8080` - Exposes websocket endpoint for the frontend
* `localhost:1935` - Exposes RTMP ingest endpoint

## Frontend
```sh
cd webapp && python -m http.server
```

## Backend
```sh
cargo run
```
