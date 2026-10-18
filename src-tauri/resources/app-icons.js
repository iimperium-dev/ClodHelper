// Иконки программ для Clod Helper (macOS): путь к .app → PNG 64×64 в base64.
// NSWorkspace.iconForFile отдаёт настоящую иконку любой программы, в том числе
// из Assets.car (у современных приложений .icns может не быть).
// Запуск: osascript -l JavaScript - <путь.app>... < app-icons.js
ObjC.import('AppKit');
function run(argv) {
  var out = {};
  argv.forEach(function (p) {
    var img = $.NSWorkspace.sharedWorkspace.iconForFile(p);
    var size = 64;
    var rep = $.NSBitmapImageRep.alloc.initWithBitmapDataPlanesPixelsWidePixelsHighBitsPerSampleSamplesPerPixelHasAlphaIsPlanarColorSpaceNameBytesPerRowBitsPerPixel(null, size, size, 8, 4, true, false, $.NSCalibratedRGBColorSpace, 0, 0);
    $.NSGraphicsContext.saveGraphicsState;
    $.NSGraphicsContext.setCurrentContext($.NSGraphicsContext.graphicsContextWithBitmapImageRep(rep));
    img.drawInRectFromRectOperationFraction($.NSMakeRect(0, 0, size, size), $.NSZeroRect, $.NSCompositingOperationCopy, 1.0);
    $.NSGraphicsContext.restoreGraphicsState;
    var png = rep.representationUsingTypeProperties($.NSBitmapImageFileTypePNG, $({}));
    out[p] = png.base64EncodedStringWithOptions(0).js;
  });
  return JSON.stringify(out);
}
