# Optimizing bandwidth

The quality of a video stream is a function of how much CPU/GPU time
you want to spend, the target bitrate, and the desired quality.

This page contains a description of some of the options for different
encoders in OBS and how they affect your stream.

## OBS settings

OBS allows you to configure what video encoder to use and tweak some
options regarding the encoding. The encoder and options you choose
directly influences the video quality and bandwidth usage of your
stream.

Most of these settings are hidden behind an "advanced" option.

![The "advanced" stream settings in OBS](/static/obs-stream-settings.png)


## General options

These are some options which are usually found on all encoders.

#### Rate control

**Recommended rate control value: VBR**

**Recommended CRF value: 18-23 (high quality)**

Most encoders expose the _rate control_ option which determines the
bitrate strategy that the encoder will use.

In conjunction with the rate control there is also the _bitrate_
setting. This determines the upper ceiling of the bandwidth usage of
your stream, but not necessarily the video _quality_. A small
resolution, or lower framerate stream will require less bitrate to
have a "good quality" compared to a big resolution, high framerate
stream.

Although the bitrate you need depends on other factors, you should
keep in mind that qwer.ee does not processing to the stream you send
to the ingest server, and directly passes it to the viewers.

This means that by setting the bitrate too high you may exclude some
viewers who do not have good enough download speed to watch your
stream.

There are a few rate control options:

##### Constant Bitrate (CBR)

This option will attempt to always keep the stream a constant bitrate,
regardless of how big the frames are (eg. if a frame is too small for
the target bitrate, it's padded with 0's).

This option is often recommended by streaming services as it's seen as
more stable to differences in network conditions, but it can be
extremely wasteful as qwer.ee does not perform any transcoding on the
server side.

##### Variable Bitrate (VBR)

**This is the recommended setting**

This option will simply try to encode frames according to the
specified video quality, while keeping the provided bitrate in mind as
the upper ceiling of the stream bitrate.

This is often combined with the _CRF_ setting. The _CRF_ option can be
seen as specifying the _video quality_ of the stream. A lower _CRF_
value will mean better quality, and higher bitrate. A high _CRF_ value
will mean both lower quality and bitrate. The default _CRF_ value will
usually be good enough.

Using the VBR rate control option will significantly lower bandwidth
usage especially in still areas.

#### Keyframe interval

**Recommended value: 5-10s**

Video codecs have a concept of _keyframe_. A video frame can either be
a keyframe or not. The reason for this concept is that designating
frames as a _non-keyframe_ allows the encoder to store less
information in the frame, since it depends on previous frame data.

The keyframe interval setting determines how often a keyframe frame is
emitted. The effect of this interval is that when you open a stream,
the viewer has to wait for the next keyframe to be transmitted.

The amount of keyframes transmitted greatly influences the bandwidth
usage of a stream, so keeping the interval larger than normal will
lower the bandwidth usage.

## VAAPI

VAAPI is the Linux interface for hardware encoding. It exposes the following options:

##### VAAPI Codec

This should be set to `H.264`

## x264

libx264 is _the_ encoder for H.264, and is very good.

##### CPU Usage Preset

Determines how much CPU time to spend on encoding. Depends on how
powerful your PC is and the quality you want to achieve, but in
general the "fast" options produce pretty good quality and keeps
processing time low.

##### Tune

Tunes the encoder according to some profile. The most notable option
here is the `zerolatency` option, which will give you the lowest
latency both for encoding and decoding at the expense of slightly more
bandwidth usage.

##### x264 Options

Free-form options that are passed on to libx264. This should include
at least `bframes=0`, to disable B-frames.
