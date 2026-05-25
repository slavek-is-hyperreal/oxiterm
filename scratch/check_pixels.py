from PIL import Image
import sys

def check_image(path):
    try:
        img = Image.open(path)
        colors = img.getcolors(maxcolors=10000)
        print(f"Image: {path}")
        print(f"Size: {img.size}")
        if colors:
            print(f"Unique colors count: {len(colors)}")
            for count, color in sorted(colors, reverse=True)[:10]:
                print(f"  {count} pixels of color {color}")
        else:
            print("More than 10000 unique colors")
    except Exception as e:
        print(f"Error reading {path}: {e}")

if __name__ == '__main__':
    for path in sys.argv[1:]:
        check_image(path)
