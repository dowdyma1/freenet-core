// automatically generated by the FlatBuffers compiler, do not modify

import * as flatbuffers from 'flatbuffers';



export class RandomBytesRequest implements flatbuffers.IUnpackableObject<RandomBytesRequestT> {
  bb: flatbuffers.ByteBuffer|null = null;
  bb_pos = 0;
  __init(i:number, bb:flatbuffers.ByteBuffer):RandomBytesRequest {
  this.bb_pos = i;
  this.bb = bb;
  return this;
}

static getRootAsRandomBytesRequest(bb:flatbuffers.ByteBuffer, obj?:RandomBytesRequest):RandomBytesRequest {
  return (obj || new RandomBytesRequest()).__init(bb.readInt32(bb.position()) + bb.position(), bb);
}

static getSizePrefixedRootAsRandomBytesRequest(bb:flatbuffers.ByteBuffer, obj?:RandomBytesRequest):RandomBytesRequest {
  bb.setPosition(bb.position() + flatbuffers.SIZE_PREFIX_LENGTH);
  return (obj || new RandomBytesRequest()).__init(bb.readInt32(bb.position()) + bb.position(), bb);
}

data():number {
  const offset = this.bb!.__offset(this.bb_pos, 4);
  return offset ? this.bb!.readUint32(this.bb_pos + offset) : 0;
}

static startRandomBytesRequest(builder:flatbuffers.Builder) {
  builder.startObject(1);
}

static addData(builder:flatbuffers.Builder, data:number) {
  builder.addFieldInt32(0, data, 0);
}

static endRandomBytesRequest(builder:flatbuffers.Builder):flatbuffers.Offset {
  const offset = builder.endObject();
  return offset;
}

static createRandomBytesRequest(builder:flatbuffers.Builder, data:number):flatbuffers.Offset {
  RandomBytesRequest.startRandomBytesRequest(builder);
  RandomBytesRequest.addData(builder, data);
  return RandomBytesRequest.endRandomBytesRequest(builder);
}

unpack(): RandomBytesRequestT {
  return new RandomBytesRequestT(
    this.data()
  );
}


unpackTo(_o: RandomBytesRequestT): void {
  _o.data = this.data();
}
}

export class RandomBytesRequestT implements flatbuffers.IGeneratedObject {
constructor(
  public data: number = 0
){}


pack(builder:flatbuffers.Builder): flatbuffers.Offset {
  return RandomBytesRequest.createRandomBytesRequest(builder,
    this.data
  );
}
}