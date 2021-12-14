const path = require('path');
const MiniCssExtractPlugin = require('mini-css-extract-plugin')
const CopyWebpackPlugin = require('copy-webpack-plugin')

module.exports = {
    entry: {
        index: './src/index.js',
        index2: './src/index2.js',
    },
    optimization: {
        minimize: false
    },
    //devtool: 'source-map',
    output: {
        pathinfo: true,
        path: path.resolve(__dirname, 'dist'),
        filename: 'js/[name].bundle.js'
    },
    module: {
        rules: [{
            test: /\.scss$/,
            use: [
                MiniCssExtractPlugin.loader,
                {
                    loader: 'css-loader',
                    options: { sourceMap: true},
                },
                {
                    loader: 'sass-loader',
                    options: {
                        sourceMap: true,
                    }
                }
            ]
        }]
    },
    plugins: [
        new CopyWebpackPlugin({
            patterns: [ { from: 'resources' } ]
        }),
        new MiniCssExtractPlugin({
            filename: 'css/mystyles.css'
        }),
    ]
};
