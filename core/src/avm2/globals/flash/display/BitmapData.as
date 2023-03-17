package flash.display {
    import flash.geom.Rectangle;
    import flash.geom.ColorTransform;
    import flash.geom.Point;
    import flash.geom.Matrix;
    import flash.filters.BitmapFilter;
    import flash.utils.ByteArray;
    import __ruffle__.stub_method;

    [Ruffle(InstanceAllocator)]
    public class BitmapData implements IBitmapDrawable {
        // FIXME - the first two arguments should not be defaulted, but it's currently
        // nedded for BitmapData to be contructed internally
        public function BitmapData(width:int=0, height:int=0, transparent:Boolean = true, fillColor:uint = 0xFFFFFFFF) {
            this.init(width, height, transparent, fillColor);
        }

        private native function init(width:int, height:int, transparent:Boolean, fillColor:uint);

        public native function get height():int;
        public native function get width():int;
        public native function get rect():Rectangle;
        public native function get transparent():Boolean;

        public native function getPixels(rect:Rectangle):ByteArray;
        public native function getVector(rect:Rectangle):Vector.<uint>;
        public native function getPixel(x:int, y:int):uint;
        public native function getPixel32(x:int, y:int):uint;
        public native function setPixel(x:int, y:int, color:uint):void;
        public native function setPixel32(x:int, y:int, color:uint):void;
        public native function setPixels(rect:Rectangle, inputByteArray:ByteArray):void;
        public native function copyChannel(sourceBitmapData:BitmapData, sourceRect:Rectangle, destPoint:Point, sourceChannel:uint, destChannel:uint):void;
        public native function floodFill(x:int, y:int, color:uint):void;
        public native function noise(randomSeed:int, low:uint = 0, high:uint = 255, channelOptions:uint = 7, grayScale:Boolean = false):void;
        public native function colorTransform(rect:Rectangle, colorTransform:ColorTransform):void;
        public native function getColorBoundsRect(mask:uint, color:uint, findColor:Boolean = true):Rectangle;
        public native function scroll(x:int, y:int):void;
        public native function lock():void;
        public native function hitTest(firstPoint:Point, firstAlphaThreshold:uint, secondObject:Object, secondBitmapDataPoint:Point = null, secondAlphaThreshold:uint = 1):Boolean;
        public native function unlock(changeRect:Rectangle = null):void;
        public native function copyPixels(
            sourceBitmapData:BitmapData, sourceRect:Rectangle, destPoint:Point, alphaBitmapData:BitmapData = null, alphaPoint:Point = null, mergeAlpha:Boolean = false
        ):void;
        public native function draw(
            source:IBitmapDrawable, matrix:Matrix = null, colorTransform:ColorTransform = null, blendMode:String = null, clipRect:Rectangle = null, smoothing:Boolean = false
        ):void;
        public native function drawWithQuality(
            source:IBitmapDrawable, matrix:Matrix = null, colorTransform:ColorTransform = null, blendMode:String = null, clipRect:Rectangle = null, smoothing:Boolean = false, quality:String = null
        ):void;
        public native function fillRect(rect:Rectangle, color:uint):void;
        public native function dispose():void;
        public native function applyFilter(sourceBitmapData:BitmapData, sourceRect:Rectangle, destPoint:Point, filter:BitmapFilter):void;
        public native function clone():BitmapData;
        public native function perlinNoise(
            baseX:Number, baseY:Number, numOctaves:uint, randomSeed:int, stitch:Boolean, fractalNoise:Boolean, channelOptions:uint = 7, grayScale:Boolean = false, offsets:Array = null
        ):void;
        public native function threshold(
            sourceBitmapData:BitmapData, sourceRect:Rectangle, destPoint:Point, operation:String, threshold:uint, color:uint = 0, mask:uint = 0xFFFFFFFF, copySource:Boolean = false
        ):uint;

        public function generateFilterRect(sourceRect:Rectangle, filter:BitmapFilter):Rectangle {
            stub_method("flash.display.BitmapData", "generateFilterRect");
            return sourceRect.clone();
        }
    }
}
