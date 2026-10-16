"""Add a measurement-only DLL project referencing production TerminalCore."""
from pathlib import Path
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET

source=Path(sys.argv[1]).resolve()
assert subprocess.check_output(['git','-C',str(source),'rev-parse','HEAD'],text=True).strip()=='7ff11ee107c984e236960ec948b450ccbc10187b'
destination=source/'src/cascadia/BlueberryNativeModel'
destination.mkdir(exist_ok=False)
shutil.copyfile(Path(__file__).with_name('NativeModel.cpp'),destination/'NativeModel.cpp')
namespace='http://schemas.microsoft.com/developer/msbuild/2003'
ET.register_namespace('',namespace)
tag=lambda name:'{'+namespace+'}'+name
project=ET.parse(source/'src/cascadia/UnitTests_TerminalCore/UnitTests.vcxproj')
root=project.getroot()
for name,value in [('ProjectGuid','{B138C7E3-4047-4D48-994D-67200D370646}'),('RootNamespace','BlueberryNativeModel'),
                   ('ProjectName','BlueberryNativeModel'),('TargetName','Blueberry.NativeModel')]:
    root.find('.//'+tag(name)).text=value
for parent in root.iter():
    for child in list(parent):
        if child.tag in (tag('ClCompile'),tag('ClInclude')) and child.get('Include'):
            parent.remove(child)
        if child.tag==tag('Import') and child.get('Project','').endswith('common.build.tests.props'):
            parent.remove(child)
for include in root.findall('.//'+tag('AdditionalIncludeDirectories')):
    include.text = '$(OpenConsoleDir)src\\cascadia\\TerminalCore\\Generated Files;'+(include.text or '')
group=ET.SubElement(root,tag('ItemGroup'))
compile=ET.SubElement(group,tag('ClCompile'),{'Include':'NativeModel.cpp'})
ET.SubElement(compile,tag('PrecompiledHeader')).text='NotUsing'
project.write(destination/'NativeModel.vcxproj',encoding='utf-8',xml_declaration=True)
print('Added buffer-only production TerminalCore observer DLL project')
