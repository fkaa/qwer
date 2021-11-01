'use strict';

var Stats = function () {
    var container = document.createElement( 'table' );
    container.style.cssText = 'font-size:11px;color:#fff;cursor:pointer;z-index:10000';

    return {
	dom: container,
	addPanel: function (panel) {
            container.appendChild( panel.dom );
            return panel;
        },
	addLabel: function (label) {
            container.appendChild(label.dom);
            return label;
        }
    };
};

Stats.Label = function(name, fg) {
    var row = document.createElement('tr');

    var label = document.createElement('td');
    label.style.cssText = 'font-weight:bold;font-family:sans-serif;';
    label.innerText = name;

    var value = document.createElement('td');
    value.setAttribute("colspan", "2");

    row.appendChild(label);
    row.appendChild(value);

    return {
	dom: row,
	update: function (text) {
            value.innerText = text;
	}
    };
};

Stats.Panel = function (name, scale, maxValue, updateLabel) {
    var row = document.createElement('tr');

    var label = document.createElement('td');
    label.style.cssText = 'font-weight:bold;font-family:sans-serif;';
    label.innerText = name;

    var value = document.createElement('td');
    var valueText = document.createElement('td');

    row.appendChild(label);
    row.appendChild(value);
    row.appendChild(valueText);


    var min = Infinity, max = 0, round = Math.round;
    var PR = round( window.devicePixelRatio || 1 ) ;

    var index = 0;
    var w = 160;
    var h = 16;

    var WIDTH = w * PR, HEIGHT = h * PR;

    var canvas = document.createElement( 'canvas' );
    canvas.width = WIDTH;
    canvas.height = HEIGHT;
    canvas.style.cssText = 'width:'+w+'px;height:'+h+';image-rendering:crisp-edges;';

    var context = canvas.getContext( '2d' );

    context.imageSmoothingEnabled = false;
    context.fillStyle = "black";
    context.fillRect( 0, 0, WIDTH, HEIGHT);

    value.appendChild(canvas);

    return {
        maxValues: w,
        maxValue: maxValue,
        cursor: 0,
        values: [],
	dom: row,
        calcStats: function() {
            let sum = 0, stddev = 0, min = Math.pow(2, 32), max = 0;

            for (let i = 0; i < this.values.length; i++) {
                sum += this.values[i];
                min = Math.min(min, this.values[i]);
                max = Math.max(min, this.values[i]);
            }

            let avg = sum / this.values.length;

            let variance = 0;
            for (let i = 0; i < this.values.length; i++) {
                variance += Math.pow(this.values[i] - avg, 2);
            }
            variance = variance / this.values.length;

            return { min: min, max: max, total: sum, average: avg, stddev: Math.sqrt(variance) };
        },
        push: function (value) {
            if (this.values.length >= this.maxValues) {
                if (this.cursor == 0) {
                    let stats = this.calcStats();
                    this.label = updateLabel(stats.min, stats.max, stats.total, stats.average, stats.stddev);
                }

                this.values[this.cursor] = value;
                this.cursor = (this.cursor + 1) % this.maxValues;
            } else {
                this.values.push(value);
            }

            this.update(value, this.maxValue);
        },
        skip: function() {
            this.update(0, this.maxValue);
        },
        getImageData: function() {
            let context = canvas.getContext("2d");
            return context.getImageData(0, 0, canvas.width, canvas.height);
        },
	update: function (value, maxValue) {
            function getPaintedColumn(value, scale) {
                var base = Math.floor(value * scale.length);
                var index = Math.ceil(value * scale.length);

                return {
                    baseIndex: Math.max(base-1, 0),
                    index: base,
                    height: Math.min(value * scale.length, 1),
                    partialHeight: (value - (base / scale.length)) * scale.length
                };
            }


            if (value > 0) {
                min = Math.min( min, value );
                max = Math.max( max, value );

                /*context.fillStyle = bg;
                  context.globalAlpha = 1;
                  context.fillRect( 0, 0, WIDTH, HEIGHT );
                  context.fillStyle = fg;*/

                var factor = value / maxValue;
                var col = getPaintedColumn(factor, scale);

                context.fillStyle = "black";
                context.fillRect(index, 0, 1, HEIGHT);

                var hh = col.height * HEIGHT;
                context.fillStyle = scale[col.baseIndex];
                context.fillRect(index, HEIGHT - hh, 1, hh);

                var hh2 = col.partialHeight * HEIGHT;
                context.fillStyle = scale[col.index];
                context.fillRect(index, HEIGHT - hh2, 1, hh2);

            }

            if (this.label) {
                valueText.innerText = this.label;
            }

            context.clearRect(index + 1, 0, 1, HEIGHT);

            index = (index + 1) % WIDTH;
	}
    };
};
