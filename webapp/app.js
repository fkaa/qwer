HOSTNAME=window.location.hostname;
WS_ADDRESS="ws://"+HOSTNAME+":8080/transport/mse/test";
WS_ADDRESS2="ws://"+HOSTNAME+":8080/transport/mse/test2";

VideoStream = function(videoElement, streamUri) {
    this.source = new MediaSource();

	var container = document.createElement('div');
	container.style.cssText = 'position:relative;display:inline-block;margin:auto;max-width:100%;';

	videoElement.style.cssText = 'max-width:100%;';

    videoElement.parentNode.appendChild(container);
    container.appendChild(videoElement);

    this.statsContainer = document.createElement('div');
    this.statsContainer.style.cssText = 'opacity:0.9;position:absolute;top:5px;left:5px;background-color:#222;padding:5px;';
    container.appendChild(this.statsContainer);

    var stats = new Stats();

    this.frameStats = stats.addLabel(new Stats.Label('Frames', '#fff'));

    var greenScale = [
        '#DEEDCF',
        '#74c67a',
        '#1d9a6c',
        '#137177',
        '#0a2f51'
    ];

    this.bufferPanel = stats.addPanel(new Stats.Panel('Buffer', greenScale));
    this.networkPanel = stats.addPanel(new Stats.Panel('Network', greenScale));
    this.fpsPanel = stats.addPanel(new Stats.Panel('Framerate', greenScale));

    this.statsContainer.appendChild(stats.dom);


    this.video = videoElement;
    this.video.src = URL.createObjectURL(this.source);

    this.video.onwaiting = function(ev) {
        console.warn("waiting!");
    }

    this.video.onstalled = function(ev) {
        console.warn("stalled!");
    }
    this.video.onsuspend = function(ev) {
        console.warn("suspend!");
    }
    this.video.onended = function(ev) {
        console.warn("ended!");
    }

    this.video.onerror = function(ev) {
        console.error(ev);
    };
    this.source.onerror = function(ev) {
        console.error(ev);
    };
    this.source.onabort = function(ev) {
        console.error(ev);
    };

    // stats
    this.networkBytes = 0;

    this.frames = [];
    this.inFlight = false;
    this.videoStarted = false;
    this.targetBuffer = 0.1;
    this.hasInit = false;

    this.source.onsourceopen = this.onsourceopen;

    this.ws = new WebSocket(streamUri);
    this.ws.binaryType = "arraybuffer";
    var onopen = function() {
        this.ws.send("start");
    };

    this.ws.onopen = onopen.bind(this);
    this.ws.onmessage = this.onwsmessage.bind(this);

    this.statsInterval = setInterval(this.gatherStats.bind(this), VideoStream.STATS_INTERVAL);
}

VideoStream.STATS_INTERVAL = 250;

VideoStream.MSG_INIT = 0;
VideoStream.MSG_FRAME = 1;

VideoStream.prototype.feedFrame = function() {
    if (this.buffer != null && !this.inFlight) {
        var frame = this.frames.shift();
        if (frame) {
            this.inFlight = true;

            this.buffer.appendBuffer(frame);
        }
    }
}

VideoStream.prototype.start = function() {
    this.ws.send("start");
}

VideoStream.prototype.gatherStats = function() {
    var buffered = this.video.buffered;
    var bufferedDuration = 0;
    var bufferStart = this.video.currentTime;
    if (buffered.length >= 1) {
        var start = buffered.start(0);
        var end = buffered.end(buffered.length - 1);

        if (start > bufferStart) { bufferStart = start; }
        bufferedDuration = end - bufferStart;
    }

    var quality = this.video.getVideoPlaybackQuality();

    var stats = {
        dropped: quality.droppedVideoFrames,
        total: quality.totalVideoFrames,
        width: this.video.videoWidth,
        height: this.video.videoHeight,
        buffered: bufferedDuration,
    };

    // basic catch-up:
    var target = this.targetBuffer;

    if (bufferedDuration > target + 1) {
        this.video.playbackRate = 2;
    } else if (bufferedDuration > target + 0.1) {
        this.video.playbackRate = 1.01;
    } else if (bufferedDuration < target - 0.5) {
        this.video.playbackRate = 0.98;
    } else {
        this.video.playbackRate = 1;
    }

    var rate = this.video.playbackRate;

    this.frameStats.update(stats.dropped + " (D) / " + stats.total + ". Buf: " + target * 1000 + "ms, Rate: " + rate );
    this.bufferPanel.update(stats.buffered * 1000, 5000, Math.round(stats.buffered * 1000) + " ms");
    this.networkPanel.update(this.networkBytes / 1024, 256, (1000 / VideoStream.STATS_INTERVAL) * Math.round(this.networkBytes / 1024) + " Kb/s");

    var fps = this.framesReceived * (1000 / VideoStream.STATS_INTERVAL);
    this.fpsPanel.update(fps, 60, fps + " fps");

    this.framesReceived = 0;
    this.networkBytes = 0;
}

VideoStream.prototype.toggleStats = function() {
    var visible = this.statsContainer.display != 'none';
    this.statsContainer.display = visible ? 'none' : 'inline-block';
}

VideoStream.prototype.setTargetBuffer = function(target) {
    this.targetBuffer = target;
}

VideoStream.prototype.onwsinit = function(init) {
    var profile = init[0];
    var constraints = init[1];
    var level = init[2];

    var codec = "video/mp4; codecs=\"avc1." + tohex(profile) + tohex(constraints) + tohex(level) + "\"";

    console.log("codec: " + codec);

    this.buffer = this.source.addSourceBuffer(codec);
    this.buffer.mode = "sequence";
    this.buffer.onerror = function(ev) {
        console.error(ev);
    };
    this.buffer.onupdateend = this.onupdateend.bind(this);
    this.feedInterval = setInterval(function() {
        this.feedFrame();
    }.bind(this), 1000/60);
}

VideoStream.prototype.onwsframe = function(frame) {
    this.framesReceived += 1;

    this.frames.push(frame);
    this.feedFrame();
}

VideoStream.prototype.onsourceopen = function(event) {
}

VideoStream.prototype.onupdateend = function(event) {
{
    var buffered = this.video.buffered;
    var bufferedDuration = 0;
    var bufferStart = this.video.currentTime;
    if (buffered.length >= 1) {
        var start = buffered.start(0);
        var end = buffered.end(buffered.length - 1);

        if (start > bufferStart) { bufferStart = start; }
        bufferedDuration = end - bufferStart;
    }

    if (this.prevTime == this.video.currentTime && bufferedDuration < this.targetBuffer) {
        //console.log("Pausing to catch up");
        //this.video.pause();
        this.stalled = this.stalled + 1;
    } else {
        console.log("Stalled for " + this.stalled);
        this.stalled = 0;
        if (this.video.paused) {
            //console.log("Unpausing to catch up");
            //this.video.play();
        }
    }

    this.prevTime = this.video.currentTime;

    //console.log("time: " + this.video.currentTime + ", buffered: " + bufferedDuration);
}



    this.inFlight = false;

    if (!this.videoStarted && bufferedDuration > this.targetBuffer) {
        console.log("Starting");

        this.video.play()
        this.videoStarted = true;
    }

    this.feedFrame();
}

// websocket callback
VideoStream.prototype.onwsmessage = function(event) {
    var bytes = new Uint8Array(event.data);

    this.networkBytes += bytes.length;

    if (!this.hasInit) {
        this.onwsinit(bytes);
        this.hasInit = true;
    } else {
        this.onwsframe(bytes);
    }

    /*var type = bytes[0];

    // console.log("type: " + type);

    var rest = bytes.subarray(1);

    switch (type) {
        case VideoStream.MSG_INIT:

            break;

        case VideoStream.MSG_FRAME:
            this.onwsframe(rest);
            break;
    }*/
}

function tohex(num) {
    return num.toString(16).padStart(2, "0");
}


VideoStream.prototype.onvideoerror = function(event) {
}
VideoStream.prototype.onsourcebuffererror = function(event) {
}

