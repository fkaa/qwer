'use strict';

function copyImage(canvas, ctx, dstWidth, dstHeight, image, srcWidth, srcHeight) {
    let newWidth = dstWidth;
    let newHeight = dstHeight;

    if (srcWidth > srcHeight) {
        newHeight = (dstWidth / srcWidth) * srcHeight;
    } else {
        newWidth = (dstHeight / srcHeight) * srcWidth;
    }

    canvas.width = newWidth;
    canvas.height = newHeight;

    ctx.drawImage(
        image,
        0,
        0,
        srcWidth,
        srcHeight,
        0,
        0,
        newWidth,
        newHeight);
}

class VideoDecoder {
    constructor(dstWidth, dstHeight) {
        this.dstWidth = dstWidth;
        this.dstHeight = dstHeight;

        this.canvas = document.createElement("canvas");
        this.canvas.width = dstWidth;
        this.canvas.height = dstHeight;
        this.context = this.canvas.getContext("2d");
        this.context.imageSmoothingEnabled = false;

        this.video = document.createElement("video");
        this.video.muted = true;

        // TODO: maybe remove this? ask jokler
        this.video.crossOrigin = "anonymous";
    }

    decode(url) {
        this.video.src = url;

        let p = new Promise(resolve => {
            this.video.onpause = () => {
                copyImage(this.canvas, this.context, this.dstWidth, this.dstHeight, this.video, this.video.videoWidth, this.video.videoHeight);

                resolve(this.canvas);
            };

            // apparently this is needed to reliably get a frame that
            // can be copied to a canvas
            this.video.play().then(() => this.video.pause());
        });

        return p.then(v => v.toDataURL());
    }
}

module.exports = VideoDecoder;
