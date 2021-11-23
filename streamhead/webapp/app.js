'use strict';

class DebugLog {
    record = false;
    records = [];
    recordCursor = 0;
    logCount = 0;
    maxRecords;
    maxFirstRecords;

    constructor(maxRecords) {
        this.maxFirstRecords = 100;
        this.maxRecords = maxRecords + this.maxFirstRecords;
        this.start = new Date();
    }

    getTime() {
        let now = new Date();
        let diff = now - this.start;

        return diff / 1000;
    }

    addLog(category, msg) {
        let time = this.getTime();

        if (this.records.length >= this.maxRecords) {
            this.records[this.maxFirstRecords + this.recordCursor] = { n: this.logCount, category: category, time: time, msg: msg };
            this.recordCursor = ((this.recordCursor + 1) % (this.maxRecords - this.maxFirstRecords));
        } else {
            this.records.push({ n: this.logCount, category: category, time: time, msg: msg });
        }

        this.logCount++;
    }

    debug(msg) {
        this.addLog("dbg", msg);
        console.log(msg);
    }

    warn(msg) {
        this.addLog("wrn", msg);
        console.warn(msg);
    }

    error(msg) {
        this.addLog("err", msg);
        console.error(msg);
    }

    getLog() {
        let log = "";

        function formatRecord(l) {
            return `${l.n.toString().padEnd(5)} ${l.category} [${l.time.toPrecision(5).padEnd(5)}] ${l.msg}\n`;
        }

        this.records.slice(0, this.maxFirstRecords).forEach(l => log += formatRecord(l));

        if (this.logCount >= this.maxFirstRecords) {
            this.records
                .slice(this.recordCursor)
                .forEach(l => log += formatRecord(l));
            this.records
                .slice(this.maxFirstRecords, this.recordCursor)
                .forEach(l => log += formatRecord(l));
        }

        return log;
    }
}

let LOG = new DebugLog(5000);

class StreamStatistics {
    #parent;
    #statsContainer;
    #frameStats;
    #bufferPanel;
    #networkPanel;
    #fpsPanel;
    #stream;

    #graphImages;

    constructor(stream) {
        this.stream = stream;
        this.frames = [];
        this.graphImages = [[], [], []];
    }

    set statsParent(parent) {
        this.deleteStatsContainer();
        this.parent = parent;
        this.createStatsContainer();
    }

    get statsParent() {
        return this.parent;
    }

    refreshStatsContainer() {
        this.deleteStatsContainer();
        this.createStatsContainer();
    }

    deleteStatsContainer() {
        if (this.parent == null) {
            return;
        }

        this.stream.onframe = null;
        this.parent.removeChild(this.statsContainer);
        clearInterval(this.updateInterval);
    }

    createStatsContainer() {
        if (this.parent == null) {
            return;
        }

        this.stream.onframe = this.onFrame.bind(this);
        this.statsContainer = document.createElement('div');
        this.statsContainer.classList.add("video-stats");
        this.statsContainer.style.cssText = 'opacity:0.9;position:absolute;top:5px;left:5px;background-color:#222;padding:5px;';
        this.parent.appendChild(this.statsContainer);

        var stats = new Stats();

        this.videoCodec = stats.addLabel(new Stats.Label('Codec', '#fff'));
        this.frameStats = stats.addLabel(new Stats.Label('Frames', '#fff'));

        var greenScale = [
            '#DEEDCF',
            '#74c67a',
            '#1d9a6c',
            '#137177',
            '#0a2f51'
        ];

        var redScale = [
            //'#ff0a54',
            //'#ff5c8a',
            //'#ff7096',
            //'#ff99ac',
            //'#f9bec7',
            '#fae0e4'
        ];

        this.bufferPanel = stats.addPanel(
            new Stats.Panel(
                'Buffer',
                greenScale,
                5000,
                (min, max, total, avg, stddev) => {
                    this.saveGraphImages();
                    return `${Math.round(avg)} ms, min: ${Math.round(min)}, max: ${Math.round(max)}, std-dev: ${stddev.toPrecision(2)} ms`
                }));
        this.networkPanel = stats.addPanel(
            new Stats.Panel(
                'Network',
                redScale,
                1024*8,
                (min, max, total, avg, stddev) => {
                    return `${Math.round((total / 1024) * 8)} Kbit/s, std-dev: ${((stddev / 1024) * 8).toPrecision(4)} Kbit`;
                }));
        this.fpsPanel = stats.addPanel(
            new Stats.Panel(
                'Framerate',
                greenScale,
                60,
                (min, max, total, avg, stddev) => {
                    return `${Math.round(1000 / avg)} FPS, std-dev: ${stddev.toPrecision(2)} ms`
                }));

        this.statsContainer.appendChild(stats.dom);

        this.updateInterval = setInterval(() => {
            this.updateStatsDom();
        }, 1000/5);
    }

    onFrame(frame) {
        this.frames.push({ t: performance.now(), frame: frame, buffered: this.stream.getBufferedVideoDuration() * 1000 });
    }

    saveGraphImages() {
        let graphs = [this.bufferPanel, this.networkPanel, this.fpsPanel];

        for (let i = 0; i < graphs.length; i++) {
            let dst = this.graphImages[i];
            if (dst.length > 30) {
                dst.shift();
            }
            dst.push(graphs[i].getImageData());
        }
    }

    getGraphImages() {
        let length = this.graphImages[0].length;
        let canvas = document.createElement("canvas");
        canvas.width = length * 160;
        canvas.height = 3 * 20;
        let context = canvas.getContext("2d");
        context.fillStyle = "black";
        context.fillRect(0, 0, canvas.width, canvas.height);

        let y = 0;
        for (let i = 0; i < this.graphImages.length; i++) {
            let src = this.graphImages[i];
            let x = 0;

            for (let j = 0; j < src.length; j++) {
                context.putImageData(src[j], x, y)
                x += 160;
            }

            y += 20;
        }

        return canvas.toDataURL();
    }

    updateStatsDom() {
        if (this.stream.videoElement == null) {
            return;
        }

        let frames = this.frames ?? [];
        this.frames = [];

        if (frames.length == 0) {
            this.bufferPanel.push(this.stream.getBufferedVideoDuration() * 1000);
            this.networkPanel.skip();
            this.fpsPanel.skip();
        }

        frames.forEach((frame) => {
            let duration = frame.t - this.lastTime;

            this.bufferPanel.push(frame.buffered);
            this.networkPanel.push(frame.frame.length);
            this.fpsPanel.push(duration);

            this.lastTime = frame.t;
        });
        let quality = this.stream.videoElement.getVideoPlaybackQuality();
        let rate = this.stream.videoElement.playbackRate;
        let target = this.stream.targetBuffer;

        let total = quality.totalVideoFrames;
        let dropped = quality.droppedVideoFrames;

        this.videoCodec.update(this.stream.codec);
        this.frameStats.update(`${dropped} (D) / ${total}. Buf: ${target * 1000} ms, Rate: ${rate}`);
        // this.bufferPanel.push(this.stream.getBufferedVideoDuration());
        // this.bufferPanel.push(this.stream.getBufferedVideoDuration());
        //this.bufferPanel.update(stats.buffered * 1000, 5000, `${Math.round(stats.buffered * 1000)} ms`);
        //this.networkPanel.update(this.networkBytes / 1024, 256, (1000 / VideoStream.STATS_INTERVAL) * Math.round(this.networkBytes / 1024) + " Kb/s");

        //var fps = this.framesReceived * (1000 / VideoStream.STATS_INTERVAL);
        //this.fpsPanel.update(fps, 60, fps + " fps");

        this.framesReceived = 0;
        this.networkBytes = 0;
    }

    stats() {
        var buffered = this.stream.getBufferedVideoDuration();

        /*var quality = this.video.getVideoPlaybackQuality();

        var stats = {
            dropped: quality.droppedVideoFrames,
            total: quality.totalVideoFrames,
            width: this.video.videoWidth,
            height: this.video.videoHeight,
            buffered: bufferedDuration,
        };*/

    }
}

class MseStream {
    #statsContainer;
    #videoElement;
    #streamUri;

    // WebSocket stuff
    #webSocket;
    #hasStartedStream;
    #isExpectingData;

    // MSE stuff
    #mseSource;
    #mseBuffer;
    #frames;
    #hasInFlightUpdates;
    #targetBuffer;
    #videoStarted;
    #previousBufferRemoval;


    #stats

    // Events
    onconnectionfail;
    onvideochanged
    onframe;

    constructor(streamUri, options) {
        this.streamUri = streamUri;
        this.videoStarted = false;
        this.hasStartedStream = false;
        this.hasInFlightUpdates = false;
        this.target = 0.5;
        this.stats = new StreamStatistics(this);
        this.frames = [];
        this.previousBufferRemoval = performance.now();
    }

    set targetBuffer(target) {
        LOG.debug(`Target buffer set to ${target}`);
        this.target = target;
    }

    get targetBuffer() {
        return this.target;
    }

    get video() {
        return this.videoElement;
    }

    set video(videoElement) {
        this.removeStream();
        this.videoElement = videoElement;
        this.attachStream();
    }

    set statsContainer(containerElement) {
        this.stats.statsParent = containerElement;
    }

    get statsContainer() {
        return this.stats.statsParent;
    }

    reconnect() {
        this.removeStream();
        this.attachStream();
    }

    getDebugLogs() {
        LOG.debug("Generating debug logs");

        let logs = `Graphs:\n${this.stats.getGraphImages()}\n\nLogs:\n${LOG.getLog()}`;

        return logs;
    }

    attachStream() {
        this.eventController = new AbortController();
        let signal = this.eventController.signal;

        LOG.debug(`Connecting to '${this.streamUri}'`);


        this.videoElement.addEventListener("playing", (e) => LOG.debug("playing"), { signal: signal });
        this.videoElement.addEventListener("pause", (e) => LOG.warn("pause"), { signal: signal });
        this.videoElement.addEventListener("waiting", (e) => LOG.warn("waiting"), { signal: signal });
        this.videoElement.addEventListener("stalled", (e) => LOG.warn("stalled"), { signal: signal });
        this.videoElement.addEventListener("suspend", (e) => LOG.warn("suspend"), { signal: signal });
        this.videoElement.addEventListener("error", (e) => LOG.error("error"), { signal: signal });

        this.webSocket = new WebSocket(this.streamUri);
        this.webSocket.binaryType = "arraybuffer";
        this.webSocket.addEventListener("close", this.webSocketClose.bind(this), { signal: signal });
        this.webSocket.addEventListener("error", this.webSocketError.bind(this), { signal: signal });
        this.webSocket.addEventListener("open", this.webSocketOpen.bind(this), { signal: signal });
        this.webSocket.addEventListener("message", this.webSocketMessage.bind(this), { signal: signal });

        this.stats.createStatsContainer();
    }

    removeStream() {
        LOG.debug("Removing media source");

        clearInterval(this.framePollInterval);
        clearInterval(this.playbackControlInterval);

        if (this.mseSource != null) {
            this.isExpectingData = false;
            this.hasStartedStream = false;
            this.hasInFlightUpdates = false;
            this.frames = [];
            this.webSocket.close(1000, "Shutting down stream");
            this.mseSource.endOfStream();
            this.mseSource.removeSourceBuffer(this.mseBuffer);
            this.eventController?.abort();
        }

        this.stats.deleteStatsContainer();

        this.videoStarted = false;
        this.mseSource = null;
        this.mseBuffer = null;
        this.webSocket = null;
    }

    streamFailed() {

    }

    webSocketOpen(event) {
        LOG.debug(`WebSocket connection to '${this.streamUri}' established`);

        this.isExpectingData = true;
    }

    webSocketClose(event) {
        LOG.warn(`WebSocket connection to '${this.streamUri}' closed: ${event.code}`);

        if (this.onconnectionfail != null) {
            this.onconnectionfail(event);
        }

        this.streamFailed();
    }

    webSocketError(event) {
        LOG.warn(`WebSocket connection error`);

        this.streamFailed();
    }

    webSocketMessageInit(codecs) {
        this.codec = `video/mp4; codecs="${codecs}"`;

        LOG.debug(`Received codec parameters: ${this.codec}`);

        let signal = this.eventController.signal;

        this.mseSource = new MediaSource();
        this.mseSource.addEventListener("sourceopen", this.mseSourceOpen.bind(this));
        this.mseSource.addEventListener("sourceclose", this.mseSourceClose.bind(this));

        let mseSrc = URL.createObjectURL(this.mseSource);
        this.videoElement.src = mseSrc;
    }

    mseBufferError(event) {
        LOG.error(`MSE buffer error: ${event}`);
    }

    mseSourceClose(event) {
        LOG.debug(`MSE source closed: ${event}`);
    }

    mseSourceOpen(event) {
        this.registerVideoEvents();

        let signal = this.eventController.signal;

        this.mseBuffer = this.mseSource.addSourceBuffer(this.codec);
        this.mseBuffer.mode = "sequence";
        this.mseBuffer.addEventListener("error", this.mseBufferError.bind(this), { signal: signal });
        this.mseBuffer.addEventListener("updateend", this.mseBufferUpdateEnd.bind(this), { signal: signal });


        this.playbackControlInterval = setInterval(
            () => {
                this.adjustPlaybackSpeed();
            },
            1000/5);
        this.framePollInterval = setInterval(
            () => {
                this.feedFrame();
            },
            1000/60);
    }

    mseBufferUpdateEnd(event) {
        const buffered = this.getBufferedVideoDuration();

        if (!this.videoStarted && buffered >= this.targetBuffer) {
            LOG.debug(`Starting video with ${buffered} seconds buffered`);

            this.videoElement.play();
            this.videoStarted = true;
        }

        this.hasInFlightUpdates = false;
        this.feedFrame();

        const removeLen = this.videoElement.currentTime - 60;
        const now = performance.now();
        if (!this.hasInFlightUpdates && removeLen > 0
            && (now - this.previousBufferRemoval) > (10 * 1000)) {

            this.mseBuffer.remove(0, removeLen);
            this.previousBufferRemoval = now;
            this.hasInFlightUpdates = true;
        }
    }

    webSocketSegment(segment) {
        if (this.onframe != null) {
            this.onframe(segment);
        }

        this.frames.push(segment);
        this.feedFrame();
    }

    webSocketMessage(event) {
        if (!this.isExpectingData) {
            return;
        }

        if (!this.hasStartedStream) {
            this.hasStartedStream = true;
            this.webSocketMessageInit(event.data);
        } else {
            var bytes = new Uint8Array(event.data);
            // this.networkBytes += bytes.length;
            this.webSocketSegment(bytes);
        }
    }

    registerVideoEvents() {

    }

    removeEvents() {
        this.eventController.abort();
    }

    // Gets the amount of buffered video from the current time in the
    // video
    getBufferedVideoDuration() {
        if (this.videoElement == null) {
            return 0;
        }

        let bufferedDuration = 0;
        let bufferStart = this.videoElement.currentTime;

        // The <video> tag can buffer ranges of time, but we only care
        // about the latest one
        const buffered = this.videoElement.buffered;

        if (buffered.length >= 1) {
            const start = buffered.start(0);
            const end = buffered.end(buffered.length - 1);

            if (start > bufferStart) {
                bufferStart = start;
            }

            bufferedDuration = end - bufferStart;
        }

        return bufferedDuration;
    }

    adjustPlaybackSpeed() {
        let buffered = this.getBufferedVideoDuration();
        let target = this.targetBuffer;

        let playbackRate = 1;
        if (buffered > target + 1) {
            playbackRate = 2;
        } else if (buffered > target + 0.5) {
            playbackRate = 1.09;
        } else if (buffered > target + 0.25) {
            playbackRate = 1.03;
        } else if (buffered > target + 0.1) {
            playbackRate = 1.01;
        } else if (buffered < target - 0.5) {
            playbackRate = 0.98;
        }

        this.videoElement.playbackRate = playbackRate;
    }

    feedFrame() {
        if (this.mseBuffer != null && !this.hasInFlightUpdates) {
            var frame = this.frames.shift();

            if (frame) {
                this.hasInFlightUpdates = true;
                this.mseBuffer.appendBuffer(frame);
            }
        }
    }
}
