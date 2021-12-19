# Streaming with Open Broadcast Studio (OBS)

[OBS](https://obsproject.com/) is a free program for streaming to
various streaming services, and can be used to stream to qwer.ee.

## Creating a custom stream output

When you install OBS it comes with a bunch of pre-configured stream
outputs for popular streaming services. Since we're not one of them,
we need to setup a custom stream output.

![A custom stream output for qwer.ee](/static/obs-custom-output.png)

The above picture shows how the stream output could look like:

1. First we need to select the _custom_ service
2. Specify the ingest server for qwer.ee. This is also where you
   specify the [stream visibility](stream-visibility).
3. Paste the stream key from your [account page](/account)

After configuring this you should be able to start streaming, but you should probably take a quick look at [Optimizing bandwidth](optimizing-bandwidth) in order to lower your bandwidth and make the stream more accessible for other people.
