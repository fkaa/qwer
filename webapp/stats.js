(function (global, factory) {
	typeof exports === 'object' && typeof module !== 'undefined' ? module.exports = factory() :
	typeof define === 'function' && define.amd ? define(factory) :
	(global.Stats = factory());
}(this, (function () { 'use strict';

/**
 * @author mrdoob / http://mrdoob.com/
 */

var Stats = function () {

	var mode = 0;

	var container = document.createElement( 'table' );
	container.style.cssText = 'font-size:11px;color:#fff;cursor:pointer;z-index:10000';
	/*container.addEventListener( 'click', function ( event ) {

		event.preventDefault();
		showPanel( ++ mode % container.children.length );

	}, false );*/

	//

	function addPanel(panel) {
		container.appendChild( panel.dom );
		return panel;
	}

    function addLabel(label) {
        container.appendChild(label.dom);
        return label;
    }

	var beginTime = ( performance || Date ).now(), prevTime = beginTime, frames = 0;

	//var fpsPanel = addPanel( new Stats.Panel( 'FPS', '#0ff', '#002' ) );
	//var msPanel = addPanel( new Stats.Panel( 'MS', '#0f0', '#020' ) );

	return {
		dom: container,

		addPanel: addPanel,
		addLabel: addLabel,
	};

};

Stats.Label = function(name, fg) {
    var row = document.createElement('tr');

    var label = document.createElement('td');
    label.style.cssText = 'font-weight:bold;font-family:sans-serif;';
    label.innerText = name;

    var value = document.createElement('td');

    row.appendChild(label);
    row.appendChild(value);
    
	return {
		dom: row,
		update: function (text) {
            value.innerText = text;
		}

	};
};

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

Stats.Panel = function (name, scale) {
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
    var w = 80;
    var h = 16;

	var WIDTH = w * PR, HEIGHT = h * PR;

	var canvas = document.createElement( 'canvas' );
	canvas.width = WIDTH;
	canvas.height = HEIGHT;
	canvas.style.cssText = 'width:'+w*2+'px;height:'+h+';image-rendering:crisp-edges;';

	var context = canvas.getContext( '2d' );

    context.imageSmoothingEnabled = false;
	//context.fillStyle = bg;
	//context.fillRect( 0, 0, WIDTH, HEIGHT);

    value.appendChild(canvas);

	return {

		dom: row,

		update: function (value, maxValue, text) {

            if (value > 0) {
                min = Math.min( min, value );
                max = Math.max( max, value );

                /*context.fillStyle = bg;
                context.globalAlpha = 1;
                context.fillRect( 0, 0, WIDTH, HEIGHT );
                context.fillStyle = fg;*/

                var factor = value / maxValue;
                var col = getPaintedColumn(factor, scale);

                var hh = col.height * HEIGHT;
                context.fillStyle = scale[col.baseIndex];
                context.fillRect(index, HEIGHT - hh, 1, hh);

                var hh2 = col.partialHeight * HEIGHT;
                context.fillStyle = scale[col.index];
                context.fillRect(index, HEIGHT - hh2, 1, hh2);

            }

            if (text) {
                valueText.innerText = text;
            }

            context.clearRect(index + 1, 0, 1, HEIGHT);

            index = (index + 1) % WIDTH;
		}

	};

};

return Stats;

})));
