# Stream visibility

When streaming to qwer.ee it's possible to choose whether you want to
have a public or unlisted stream.

This is determined by the ingest URL you decide to stream to. Ingest
URLs have the following format: `rtmp://ingest.qwer.ee/<app>`.

Replacing `<app>` with any of the following options determines the
stream visibility:

* `public` — The stream will show up on the [streams page](/streams)
* `unlisted` — The stream will be hidden on the [streams
  page](/streams) but still accessible to anyone with a link to the
  stream page.
* Anything not matching the above will default to the `unlisted`
  option
